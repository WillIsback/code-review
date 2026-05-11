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
from typing import Optional

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


def get_config() -> dict:
    return {
        "vllm_base_url": VLLM_BASE_URL,
        "batch_size": BATCH_SIZE,
    }


def check_dirty_tree(repo_path: Path = Path(".")) -> list[str]:
    """Return list of modified/untracked files; empty if working tree is clean."""
    repo = git.Repo(repo_path, search_parent_directories=True)
    modified = [item.a_path for item in repo.index.diff(None)]
    return modified + list(repo.untracked_files)


def check_vllm_reachable(base_url: str) -> bool:
    """Return True if the vLLM /v1/models endpoint responds."""
    try:
        url = base_url.rstrip("/")
        if url.endswith("/v1"):
            url = url[:-3]
        httpx.get(f"{url}/v1/models", timeout=VLLM_CONNECT_TIMEOUT).raise_for_status()
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


def detect_model(base_url: str) -> Optional[str]:
    """Query /v1/models and return the first available model ID."""
    try:
        url = base_url.rstrip("/")
        if url.endswith("/v1"):
            url = url[:-3]
        response = httpx.get(f"{url}/v1/models", timeout=VLLM_CONNECT_TIMEOUT)
        response.raise_for_status()
        data = response.json()
        if data.get("data"):
            model_id = data["data"][0]["id"]
            typer.echo(f"📦 Auto-detected model: {model_id}")
            return model_id
        return None
    except Exception:
        return None
