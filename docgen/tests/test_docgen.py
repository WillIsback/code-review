import sys
import os
from pathlib import Path

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "../.."))

import importlib
from unittest.mock import patch, MagicMock


class TestGetConfig:
    def test_returns_defaults_when_env_not_set(self):
        with patch.dict(os.environ, {}, clear=True):
            import docgen.docgen as m
            importlib.reload(m)
            cfg = m.get_config()
        assert cfg["batch_size"] == 4
        assert "localhost" in cfg["vllm_base_url"]

    def test_reads_vllm_base_url_from_env(self):
        with patch.dict(os.environ, {"VLLM_BASE_URL": "http://myserver:8000/v1", "BATCH_SIZE": "4"}):
            import docgen.docgen as m
            importlib.reload(m)
            cfg = m.get_config()
        assert cfg["vllm_base_url"] == "http://myserver:8000/v1"

    def test_reads_batch_size_from_env(self):
        with patch.dict(os.environ, {"VLLM_BASE_URL": "http://x:1/v1", "BATCH_SIZE": "8"}):
            import docgen.docgen as m
            importlib.reload(m)
            cfg = m.get_config()
        assert cfg["batch_size"] == 8


class TestCheckDirtyTree:
    def test_returns_empty_list_when_clean(self):
        mock_repo = MagicMock()
        mock_repo.index.diff.return_value = []
        mock_repo.untracked_files = []
        with patch("docgen.docgen.git.Repo", return_value=mock_repo):
            import docgen.docgen as m
            result = m.check_dirty_tree(Path("."))
        assert result == []

    def test_returns_modified_files_when_dirty(self):
        mock_diff = MagicMock()
        mock_diff.a_path = "src/foo.py"
        mock_repo = MagicMock()
        mock_repo.index.diff.return_value = [mock_diff]
        mock_repo.untracked_files = ["src/bar.py"]
        with patch("docgen.docgen.git.Repo", return_value=mock_repo):
            import docgen.docgen as m
            result = m.check_dirty_tree(Path("."))
        assert "src/foo.py" in result
        assert "src/bar.py" in result


class TestCheckVllmReachable:
    def test_returns_true_when_reachable(self):
        mock_response = MagicMock()
        mock_response.raise_for_status.return_value = None
        with patch("docgen.docgen.httpx.get", return_value=mock_response):
            import docgen.docgen as m
            assert m.check_vllm_reachable("http://localhost:30000/v1") is True

    def test_returns_false_on_connection_error(self):
        with patch("docgen.docgen.httpx.get", side_effect=Exception("timeout")):
            import docgen.docgen as m
            assert m.check_vllm_reachable("http://localhost:30000/v1") is False
