import sys
import os
from types import SimpleNamespace

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import reviewer

from reviewer import build_vllm_models_url, split_diff_into_chunks


class TestBuildVllmModelsUrl:
    def test_base_url_with_v1_suffix(self):
        url = build_vllm_models_url("http://192.168.1.87:30000/v1")
        assert url == "http://192.168.1.87:30000/v1/models"

    def test_base_url_without_v1_suffix(self):
        url = build_vllm_models_url("http://192.168.1.87:30000")
        assert url == "http://192.168.1.87:30000/v1/models"

    def test_base_url_with_trailing_slash(self):
        url = build_vllm_models_url("http://192.168.1.87:30000/v1/")
        assert url == "http://192.168.1.87:30000/v1/models"


class TestSplitDiffIntoChunks:
    def test_small_diff_returns_single_chunk(self):
        diff = "line one\nline two\nline three"
        chunks = split_diff_into_chunks(diff, max_tokens=1000)
        assert len(chunks) == 1
        assert chunks[0] == diff

    def test_large_diff_splits_into_multiple_chunks(self):
        # Each word counts as 1 token; 300 words per line × 10 lines = 3000 tokens
        line = " ".join(["word"] * 300)
        diff = "\n".join([line] * 10)
        chunks = split_diff_into_chunks(diff, max_tokens=500)
        assert len(chunks) > 1

    def test_empty_diff_returns_single_empty_chunk(self):
        chunks = split_diff_into_chunks("")
        assert len(chunks) == 1
        assert chunks[0] == ""


class TestReviewCode:
    def test_review_code_uses_non_thinking_payload(self, monkeypatch):
        captured = {}

        class FakeCompletions:
            def create(self, **kwargs):
                captured.update(kwargs)
                return SimpleNamespace(
                    choices=[
                        SimpleNamespace(
                            message=SimpleNamespace(
                                content="Issues\nImprovements\nPositives"
                            ),
                        )
                    ]
                )

        class FakeChat:
            completions = FakeCompletions()

        class FakeClient:
            chat = FakeChat()

        monkeypatch.setattr(reviewer, "get_openai_client", lambda: FakeClient())

        result = reviewer.review_code("+ added line", "Qwen/Qwen3.6-35B-A3B-FP8")

        assert result == "Issues\nImprovements\nPositives"
        assert captured["reasoning_effort"] == "none"
        assert captured["extra_body"] == {
            "chat_template_kwargs": {"enable_thinking": False}
        }


class TestPostReviewComment:
    def test_post_review_comment_success(self, monkeypatch):
        monkeypatch.setattr(reviewer, "GITHUB_TOKEN", "token")
        monkeypatch.setattr(reviewer, "GITHUB_REPOSITORY", "org/repo")
        monkeypatch.setattr(reviewer, "PULL_REQUEST_NUMBER", "1")
        monkeypatch.setattr(reviewer, "MAX_GITHUB_COMMENT_LEN", 60000)

        captured = {}

        class FakeResponse:
            def raise_for_status(self):
                return None

        def fake_post(url, json, headers, timeout):
            captured["url"] = url
            captured["json"] = json
            captured["headers"] = headers
            captured["timeout"] = timeout
            return FakeResponse()

        monkeypatch.setitem(sys.modules, "requests", SimpleNamespace(post=fake_post))

        ok = reviewer.post_review_comment("Looks good")

        assert ok is True
        assert captured["url"].endswith("/issues/1/comments")
        assert captured["headers"]["Authorization"] == "token token"
        assert "## 🤖 AI Code Review" in captured["json"]["body"]
        assert "Looks good" in captured["json"]["body"]

    def test_post_review_comment_truncates_body(self, monkeypatch):
        monkeypatch.setattr(reviewer, "GITHUB_TOKEN", "token")
        monkeypatch.setattr(reviewer, "GITHUB_REPOSITORY", "org/repo")
        monkeypatch.setattr(reviewer, "PULL_REQUEST_NUMBER", "1")
        monkeypatch.setattr(reviewer, "MAX_GITHUB_COMMENT_LEN", 200)

        captured = {}

        class FakeResponse:
            def raise_for_status(self):
                return None

        def fake_post(url, json, headers, timeout):
            captured["body"] = json["body"]
            return FakeResponse()

        monkeypatch.setitem(sys.modules, "requests", SimpleNamespace(post=fake_post))

        reviewer.post_review_comment("x" * 1000)

        assert len(captured["body"]) <= 200
        assert captured["body"].endswith("[Truncated due to GitHub comment size limit]")
