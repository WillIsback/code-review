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
