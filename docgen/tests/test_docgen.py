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
