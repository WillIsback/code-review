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
from datetime import datetime
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
    """Return True if source contains functions/classes that need JSDoc."""
    lines = source.splitlines()
    declaration_re = re.compile(
        r'^(?:export\s+)?(?:async\s+)?(?:function\s+\w+|class\s+\w+)'
    )
    for i, line in enumerate(lines):
        if declaration_re.match(line.strip()):
            prev_non_empty = next(
                (lines[j].strip() for j in range(i - 1, -1, -1) if lines[j].strip()),
                ""
            )
            has_jsdoc = prev_non_empty.endswith("*/")
            if not has_jsdoc or force:
                return True
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
