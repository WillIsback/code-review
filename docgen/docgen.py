#!/usr/bin/env python3
"""
Docgen CLI — generates docstrings for Python and TypeScript files
using a locally-hosted vLLM instance.
"""

import ast
import asyncio
import os
import re
import sys
from pathlib import Path
from typing import Optional, TypedDict

import git
import httpx
import typer
from dotenv import load_dotenv
from openai import AsyncOpenAI

load_dotenv()

VLLM_BASE_URL: str = os.getenv("VLLM_BASE_URL", "http://localhost:30000/v1")
BATCH_SIZE: int = int(os.getenv("BATCH_SIZE", "4"))
VLLM_CONNECT_TIMEOUT: int = 5

app = typer.Typer(help="Generate docstrings using a local vLLM instance.")


class Config(TypedDict):
    vllm_base_url: str
    batch_size: int


def get_config() -> Config:
    """Return runtime configuration from environment variables."""
    return {
        "vllm_base_url": VLLM_BASE_URL,
        "batch_size": BATCH_SIZE,
    }


def check_dirty_tree(repo_path: Path = Path(".")) -> list[str]:
    """Return list of modified/untracked files; empty if working tree is clean."""
    repo = git.Repo(repo_path, search_parent_directories=True)
    staged = [item.a_path for item in repo.index.diff(repo.head.commit)]
    unstaged = [item.a_path for item in repo.index.diff(None)]
    return staged + unstaged + list(repo.untracked_files)


def _models_url(base_url: str) -> str:
    """Normalise base_url and return the /v1/models endpoint URL."""
    url = base_url.rstrip("/")
    if url.endswith("/v1"):
        url = url[:-3]
    return f"{url}/v1/models"


def check_vllm_reachable(base_url: str) -> bool:
    """Return True if the vLLM /v1/models endpoint responds."""
    try:
        httpx.get(_models_url(base_url), timeout=VLLM_CONNECT_TIMEOUT).raise_for_status()
        return True
    except Exception:
        return False


def resolve_files(target: Path, recursive: bool = False) -> list[Path]:
    """Return .py/.ts/.tsx files under target. Flat by default, recursive if flagged."""
    extensions = {".py", ".ts", ".tsx"}
    if target.is_file():
        return [target] if target.suffix in extensions else []
    pattern = "**/*" if recursive else "*"
    return sorted(f for f in target.glob(pattern) if f.is_file() and f.suffix in extensions)


def python_has_missing_docstrings(source: str, force: bool = False) -> bool:
    """Return True if source contains functions/classes that need docstrings."""
    try:
        tree = ast.parse(source)
    except SyntaxError:
        return False
    for node in ast.walk(tree):
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
            has_doc = (
                node.body
                and isinstance(node.body[0], ast.Expr)
                and isinstance(node.body[0].value, ast.Constant)
                and isinstance(node.body[0].value.value, str)
            )
            if not has_doc or force:
                return True
    return False


def ts_has_missing_docstrings(source: str, force: bool = False) -> bool:
    """Return True if source contains functions/classes/methods that need JSDoc."""
    lines = source.splitlines()
    declaration_re = re.compile(
        r'^(?:export\s+)?(?:async\s+)?(?:function\s+\w+|class\s+\w+)'
        r'|^(?:(?:public|private|protected|static|async|override)\s+)*\w+\s*\('
    )
    for i, line in enumerate(lines):
        stripped = line.strip()
        if not stripped or stripped.startswith("//") or stripped.startswith("*"):
            continue
        if declaration_re.match(stripped):
            prev_non_empty = next(
                (lines[j].strip() for j in range(i - 1, -1, -1) if lines[j].strip()),
                ""
            )
            has_jsdoc = prev_non_empty.endswith("*/")
            if not has_jsdoc or force:
                return True
    return False


def get_format(file: Path, fmt: Optional[str]) -> str:
    """Return docstring format: explicit override or auto-detected from extension."""
    if fmt is not None:
        return fmt
    return "mkdocs" if file.suffix == ".py" else "tsdoc"


def get_language(file: Path) -> str:
    """Return human-readable language name for the file."""
    return "Python" if file.suffix == ".py" else "TypeScript"


def needs_docstrings(file: Path, force: bool = False) -> bool:
    """Return True if this file has functions/classes that need docstrings."""
    try:
        source = file.read_text()
    except OSError:
        return False
    if file.suffix == ".py":
        return python_has_missing_docstrings(source, force)
    if file.suffix in {".ts", ".tsx"}:
        return ts_has_missing_docstrings(source, force)
    return False


def detect_model(base_url: str) -> Optional[str]:
    """Query /v1/models and return the first available model ID."""
    try:
        response = httpx.get(_models_url(base_url), timeout=VLLM_CONNECT_TIMEOUT)
        response.raise_for_status()
        data = response.json()
        if data.get("data"):
            model_id = data["data"][0]["id"]
            typer.echo(f"📦 Auto-detected model: {model_id}")
            return model_id
        return None
    except Exception:
        return None


async def generate_docstrings_async(
    client: AsyncOpenAI,
    semaphore: asyncio.Semaphore,
    file: Path,
    fmt: str,
    force: bool,
    model: str,
) -> tuple[Path, Optional[str]]:
    """Send file source to vLLM and return (file, patched_source) or (file, None) on error."""
    source = file.read_text()
    language = get_language(file)
    action = (
        "Replace all existing docstrings and add missing ones"
        if force
        else "Add docstrings to all functions and classes that are missing them"
    )
    prompt = (
        f"{action} using {fmt} format in the following {language} source code. "
        f"Return ONLY the complete patched source code with no explanation and no markdown fences.\n\n"
        f"{source}"
    )
    async with semaphore:
        try:
            response = await client.chat.completions.create(
                model=model,
                messages=[{"role": "user", "content": prompt}],
                extra_body={
                    "reasoning_effort": "none",
                    "chat_template_kwargs": {"enable_thinking": False},
                },
            )
            return file, response.choices[0].message.content
        except Exception as e:
            typer.echo(f"  ⚠️  LLM error for {file.name}: {e}", err=True)
            return file, None


async def process_files_async(
    files: list[Path],
    fmt: Optional[str],
    force: bool,
    model: str,
    batch_size: int,
    base_url: str,
) -> dict[Path, str]:
    """Process files in parallel (up to batch_size concurrent LLM calls)."""
    client = AsyncOpenAI(base_url=base_url, api_key="none")
    semaphore = asyncio.Semaphore(batch_size)
    tasks = [
        generate_docstrings_async(client, semaphore, f, get_format(f, fmt), force, model)
        for f in files
    ]
    results = await asyncio.gather(*tasks)
    return {path: content for path, content in results if content is not None}


@app.command()
def main(
    target: Path = typer.Argument(..., help="File or folder to process"),
    fmt: Optional[str] = typer.Option(None, "--format", help="Docstring format: mkdocs, tsdoc (auto-detected if omitted)"),
    recursive: bool = typer.Option(False, "--recursive", "-r", help="Recurse into subdirectories"),
    force: bool = typer.Option(False, "--force", help="Regenerate existing docstrings"),
) -> None:
    config = get_config()

    dirty = check_dirty_tree()
    if dirty:
        typer.echo("✗ Working tree has uncommitted changes. Commit or stash before running docgen:")
        for f in dirty:
            typer.echo(f"  {f}")
        raise typer.Exit(1)

    if not check_vllm_reachable(config["vllm_base_url"]):
        typer.echo(f"✗ vLLM not reachable at {config['vllm_base_url']}")
        raise typer.Exit(1)

    model = detect_model(config["vllm_base_url"])
    if not model:
        typer.echo("✗ Could not detect model from vLLM server")
        raise typer.Exit(1)

    files = resolve_files(target, recursive)
    if not files:
        typer.echo("⚠️  No Python or TypeScript files found.")
        raise typer.Exit(2)

    to_process = [f for f in files if needs_docstrings(f, force)]
    if not to_process:
        typer.echo("✓ Nothing to do — all files are already documented.")
        raise typer.Exit(2)

    typer.echo(f"📝 Processing {len(to_process)} file(s) in batches of {config['batch_size']}...")
    patched = asyncio.run(
        process_files_async(to_process, fmt, force, model, config["batch_size"], config["vllm_base_url"])
    )

    if not patched:
        typer.echo("⚠️  No files were successfully processed.")
        raise typer.Exit(1)

    typer.echo("🔀 Applying changes via git branch...")
    apply_with_git(patched)
    typer.echo(f"✓ Done. Docstrings added to {len(patched)} file(s).")


def apply_with_git(patched: dict[Path, str], repo_path: Path = Path(".")) -> None:
    """Write patched files in a short-lived git branch and merge back into current branch."""
    from datetime import datetime
    repo = git.Repo(repo_path, search_parent_directories=True)
    branch_name = f"docgen/{datetime.now().strftime('%Y%m%d-%H%M%S')}"
    original_branch = repo.active_branch.name
    success = False

    repo.git.checkout("-b", branch_name)
    try:
        for path, content in patched.items():
            path.write_text(content)
        repo.git.add("--", *[str(p) for p in patched.keys()])
        repo.git.commit("-m", "docs: add docstrings via docgen")
        repo.git.checkout(original_branch)
        repo.git.merge(branch_name, "--no-ff", "-m", f"docs: merge {branch_name}")
        success = True
    except Exception as e:
        typer.echo(f"  ✗ Git error: {e}", err=True)
        typer.echo(f"  Branch '{branch_name}' left for manual inspection.", err=True)
        try:
            repo.git.checkout(original_branch)
        except Exception:
            pass
        raise
    finally:
        if success:
            try:
                repo.git.branch("-d", branch_name)
            except Exception:
                pass


if __name__ == "__main__":
    app()
