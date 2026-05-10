#!/usr/bin/env python3
"""
AI Code Reviewer for GitHub PRs using vLLM
This script connects to a local vLLM instance to review code changes.
Safe-guarded to prevent runner hangs and graceful degradation.
"""

import os
import sys
import httpx
from openai import OpenAI
from typing import Optional
from urllib.parse import urljoin

# Configuration
VLLM_BASE_URL = os.getenv("VLLM_BASE_URL", "http://localhost:30000/v1")
VLLM_TIMEOUT = int(os.getenv("VLLM_TIMEOUT", "60"))
VLLM_CONNECT_TIMEOUT = int(os.getenv("VLLM_CONNECT_TIMEOUT", "5"))
VLLM_RETRIES = int(os.getenv("VLLM_RETRIES", "2"))
MAX_GITHUB_COMMENT_LEN = int(os.getenv("MAX_GITHUB_COMMENT_LEN", "60000"))

# GitHub
GITHUB_TOKEN = os.getenv("GITHUB_TOKEN")
GITHUB_REPOSITORY = os.getenv("GITHUB_REPOSITORY")
PULL_REQUEST_NUMBER = os.getenv("PULL_REQUEST_NUMBER")


def build_vllm_models_url(base_url: str) -> str:
    """Build a stable /v1/models URL from a potentially variable base URL."""
    base = base_url.rstrip("/")
    if base.endswith("/v1"):
        base = base[:-3]
    return urljoin(f"{base}/", "v1/models")


def detect_available_model() -> Optional[str]:
    """
    Dynamically detect the available model from vLLM server.
    Queries /v1/models endpoint instead of hardcoding.
    Returns first available model ID or None if detection fails.
    """
    try:
        models_url = build_vllm_models_url(VLLM_BASE_URL)
        response = httpx.get(models_url, timeout=VLLM_CONNECT_TIMEOUT)
        response.raise_for_status()

        data = response.json()
        if data.get("data") and len(data["data"]) > 0:
            model_id = data["data"][0].get("id")
            print(f"📦 Auto-detected model: {model_id}")
            return model_id
        else:
            print("⚠️  No models found in vLLM response")
            return None

    except Exception as e:
        print(f"⚠️  Failed to detect model: {e}")
        print(f"   Falling back to manual specification (set VLLM_MODEL env var)")
        # Fall back to environment variable if set
        fallback = os.getenv("VLLM_MODEL")
        if fallback:
            print(f"   Using fallback: {fallback}")
            return fallback
        return None


def get_openai_client() -> OpenAI:
    """Initialize OpenAI client pointing to vLLM endpoint with safeguards."""
    return OpenAI(
        base_url=VLLM_BASE_URL,
        api_key="not-needed-for-local",
        max_retries=VLLM_RETRIES,
        timeout=httpx.Timeout(
            VLLM_TIMEOUT,  # Total timeout
            connect=VLLM_CONNECT_TIMEOUT,  # Fail fast if server is down
        ),
    )


def get_pr_diff() -> Optional[str]:
    """
    Get PR diff using GitHub CLI or API.
    Returns the unified diff or None if unavailable.
    """
    try:
        if GITHUB_TOKEN and GITHUB_REPOSITORY and PULL_REQUEST_NUMBER:
            # Use GitHub API
            import requests

            headers = {"Authorization": f"token {GITHUB_TOKEN}"}
            url = f"https://api.github.com/repos/{GITHUB_REPOSITORY}/pulls/{PULL_REQUEST_NUMBER}"
            files_url = f"{url}/files"

            diff_content = ""
            page = 1
            max_pages = 50
            while page <= max_pages:
                response = requests.get(
                    f"{files_url}?page={page}&per_page=100",
                    headers=headers,
                    timeout=10,
                )
                response.raise_for_status()
                files = response.json()

                if not files:
                    break

                for file in files:
                    patch = file.get("patch")
                    if patch:
                        diff_content += (
                            f"\n\n# File: {file.get('filename', 'unknown')}\n"
                        )
                        diff_content += patch

                page += 1

            if page > max_pages:
                print(
                    f"⚠️  Reached pagination cap ({max_pages} pages); diff may be incomplete"
                )

            return diff_content
        else:
            print("⚠️  GitHub credentials not available for API access")
            return None

    except Exception as e:
        print(f"❌ Failed to fetch PR diff: {e}")
        return None


def split_diff_into_chunks(diff: str, max_tokens: int = 2000) -> list[str]:
    """Split large diffs into chunks for better review."""
    lines = diff.split("\n")
    chunks = []
    current_chunk = []
    current_size = 0

    for line in lines:
        line_size = len(line.split())
        if current_size + line_size > max_tokens and current_chunk:
            chunks.append("\n".join(current_chunk))
            current_chunk = [line]
            current_size = line_size
        else:
            current_chunk.append(line)
            current_size += line_size

    if current_chunk:
        chunks.append("\n".join(current_chunk))

    return chunks


def review_code(diff_text: str, model: str) -> Optional[str]:
    """
    Review code using vLLM. Fails gracefully if vLLM is unavailable.
    Returns review text or None if failed.

    Args:
        diff_text: The PR diff to review
        model: The model ID to use for review
    """
    if not diff_text:
        print("⚠️  No diff content to review")
        return None

    try:
        client = get_openai_client()

        # Split large diffs
        chunks = split_diff_into_chunks(diff_text)
        reviews = []

        for i, chunk in enumerate(chunks):
            print(f"Reviewing chunk {i + 1}/{len(chunks)}...")

            try:
                response = client.chat.completions.create(
                    model=model,
                    messages=[
                        {
                            "role": "system",
                            "content": "You are a code reviewer. Be brief, practical, and output final answer only.",
                        },
                        {
                            "role": "user",
                            "content": f"Review this code diff. Return only Markdown sections: Issues, Improvements, Positives.\n\n```diff\n{chunk}\n```",
                        },
                    ],
                    max_tokens=350,
                    temperature=0.1,
                    reasoning_effort="none",
                    extra_body={"chat_template_kwargs": {"enable_thinking": False}},
                )

                message = response.choices[0].message
                review_text = message.content or ""
                if not review_text.strip() and hasattr(message, "reasoning"):
                    print(
                        f"   ⚠️  Chunk {i + 1}: reasoning was returned but final content was empty"
                    )

                if review_text.strip():
                    reviews.append(review_text)
                else:
                    print(f"   ⚠️  Chunk {i + 1}: No content in response")
                    reviews.append(f"Chunk {i + 1}: empty model response")

            except httpx.TimeoutException:
                print(f"⏱️  Timeout on chunk {i + 1} - vLLM server may be slow")
                reviews.append(f"Chunk {i + 1}: timeout")
                continue
            except Exception as e:
                print(f"⚠️  Error reviewing chunk {i + 1}: {e}")
                reviews.append(f"Chunk {i + 1}: error during analysis")
                continue

        return "\n\n---\n\n".join(reviews) if reviews else None

    except Exception as e:
        print(f"❌ AI Review failed: {e}")
        return None


def post_review_comment(review_text: str) -> bool:
    """
    Post the review as a PR comment using GitHub CLI or API.
    Returns True if successful, False otherwise.
    """
    if not GITHUB_TOKEN or not GITHUB_REPOSITORY or not PULL_REQUEST_NUMBER:
        print("⚠️  Missing GitHub credentials to post comment")
        return False

    try:
        # Prepare comment body
        source_label = os.getenv("REVIEW_ENGINE_LABEL", "self-hosted AI reviewer")
        comment_body = (
            "## 🤖 AI Code Review\n\n"
            f"{review_text}\n\n"
            "---\n"
            f"*This review was generated by {source_label}.*"
        )

        if len(comment_body) > MAX_GITHUB_COMMENT_LEN:
            truncation_notice = "\n\n[Truncated due to GitHub comment size limit]"
            allowed_len = MAX_GITHUB_COMMENT_LEN - len(truncation_notice)
            comment_body = comment_body[:allowed_len] + truncation_notice

        # Post via GitHub API
        import requests

        headers = {"Authorization": f"token {GITHUB_TOKEN}"}
        url = f"https://api.github.com/repos/{GITHUB_REPOSITORY}/issues/{PULL_REQUEST_NUMBER}/comments"

        response = requests.post(
            url, json={"body": comment_body}, headers=headers, timeout=10
        )
        response.raise_for_status()

        print("✅ Review comment posted successfully")
        return True

    except Exception as e:
        print(f"❌ Failed to post comment: {e}")
        return False


def main():
    """Main entry point."""
    print("=" * 60)
    print("🚀 AI Code Reviewer - Self-Hosted Runner")
    print("=" * 60)

    # Detect model dynamically
    print(f"\n🔍 Detecting available model...")
    model = detect_available_model()

    if not model:
        print("❌ Could not detect or fallback to any model")
        return 1

    # Verify environment
    print(f"\n📋 Configuration:")
    print(f"  vLLM URL: {VLLM_BASE_URL}")
    print(f"  Model: {model} (auto-detected)")
    print(f"  Timeout: {VLLM_TIMEOUT}s (connect: {VLLM_CONNECT_TIMEOUT}s)")
    print(f"  PR: {GITHUB_REPOSITORY}#{PULL_REQUEST_NUMBER}")

    # Get diff
    print(f"\n📥 Fetching PR diff...")
    diff = get_pr_diff()

    if not diff:
        print("⚠️  Could not retrieve PR diff. Skipping review.")
        return 1

    print(f"📊 Diff size: {len(diff)} characters")

    # Review code
    print(f"\n🤖 Analyzing code with vLLM...\n")
    review = review_code(diff, model)

    if not review:
        print("\n❌ Code review failed or timed out")
        return 1

    print(f"\n✅ Review completed:\n")
    print(review)

    # Post comment
    print(f"\n💬 Posting review as PR comment...")
    if post_review_comment(review):
        print("✅ Done!")
        return 0
    else:
        print("⚠️  Review generated but comment posting failed")
        return 1


if __name__ == "__main__":
    sys.exit(main())
