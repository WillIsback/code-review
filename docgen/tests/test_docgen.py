import sys
import os
import tempfile
import pytest
from pathlib import Path

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "../.."))

import importlib
import asyncio
from unittest.mock import patch, MagicMock, AsyncMock
from typer.testing import CliRunner


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

    def test_returns_staged_files_when_staged(self):
        mock_staged = MagicMock()
        mock_staged.a_path = "src/staged.py"
        mock_repo = MagicMock()
        mock_repo.index.diff.side_effect = lambda arg: [mock_staged] if arg == mock_repo.head.commit else []
        mock_repo.untracked_files = []
        with patch("docgen.docgen.git.Repo", return_value=mock_repo):
            import docgen.docgen as m
            result = m.check_dirty_tree(Path("."))
        assert "src/staged.py" in result


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


class TestDetectModel:
    def test_returns_first_model_id(self):
        mock_response = MagicMock()
        mock_response.raise_for_status.return_value = None
        mock_response.json.return_value = {"data": [{"id": "Qwen/Qwen3-30B-A3B"}]}
        with patch("docgen.docgen.httpx.get", return_value=mock_response):
            import docgen.docgen as m
            assert m.detect_model("http://localhost:30000/v1") == "Qwen/Qwen3-30B-A3B"

    def test_returns_none_when_no_models(self):
        mock_response = MagicMock()
        mock_response.raise_for_status.return_value = None
        mock_response.json.return_value = {"data": []}
        with patch("docgen.docgen.httpx.get", return_value=mock_response):
            import docgen.docgen as m
            assert m.detect_model("http://localhost:30000/v1") is None

    def test_returns_none_on_error(self):
        with patch("docgen.docgen.httpx.get", side_effect=Exception("conn refused")):
            import docgen.docgen as m
            assert m.detect_model("http://localhost:30000/v1") is None


class TestResolveFiles:
    def test_single_python_file(self):
        with tempfile.TemporaryDirectory() as d:
            f = Path(d) / "foo.py"
            f.write_text("x = 1")
            import docgen.docgen as m
            assert m.resolve_files(f) == [f]

    def test_single_non_matching_file_returns_empty(self):
        with tempfile.TemporaryDirectory() as d:
            f = Path(d) / "README.md"
            f.write_text("# docs")
            import docgen.docgen as m
            assert m.resolve_files(f) == []

    def test_flat_folder_no_recursion(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            (root / "a.py").write_text("")
            (root / "b.ts").write_text("")
            sub = root / "sub"
            sub.mkdir()
            (sub / "c.py").write_text("")
            import docgen.docgen as m
            result = m.resolve_files(root, recursive=False)
            names = {f.name for f in result}
            assert names == {"a.py", "b.ts"}

    def test_recursive_folder(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            (root / "a.py").write_text("")
            sub = root / "sub"
            sub.mkdir()
            (sub / "c.py").write_text("")
            import docgen.docgen as m
            result = m.resolve_files(root, recursive=True)
            names = {f.name for f in result}
            assert names == {"a.py", "c.py"}

    def test_tsx_files_included(self):
        with tempfile.TemporaryDirectory() as d:
            f = Path(d) / "Button.tsx"
            f.write_text("")
            import docgen.docgen as m
            assert m.resolve_files(f) == [f]


class TestPythonHasMissingDocstrings:
    def test_function_without_docstring_returns_true(self):
        source = "def foo():\n    return 1\n"
        import docgen.docgen as m
        assert m.python_has_missing_docstrings(source) is True

    def test_function_with_docstring_returns_false(self):
        source = 'def foo():\n    """Does foo."""\n    return 1\n'
        import docgen.docgen as m
        assert m.python_has_missing_docstrings(source) is False

    def test_class_without_docstring_returns_true(self):
        source = "class Bar:\n    pass\n"
        import docgen.docgen as m
        assert m.python_has_missing_docstrings(source) is True

    def test_class_with_docstring_returns_false(self):
        source = 'class Bar:\n    """Bar class."""\n    pass\n'
        import docgen.docgen as m
        assert m.python_has_missing_docstrings(source) is False

    def test_force_true_returns_true_even_when_documented(self):
        source = 'def foo():\n    """Has docstring."""\n    return 1\n'
        import docgen.docgen as m
        assert m.python_has_missing_docstrings(source, force=True) is True

    def test_invalid_syntax_returns_false(self):
        source = "def foo(:\n    pass\n"
        import docgen.docgen as m
        assert m.python_has_missing_docstrings(source) is False

    def test_no_functions_or_classes_returns_false(self):
        source = "x = 1\ny = 2\n"
        import docgen.docgen as m
        assert m.python_has_missing_docstrings(source) is False


class TestTsHasMissingDocstrings:
    def test_function_without_jsdoc_returns_true(self):
        source = "function greet(name: string): string {\n  return name;\n}\n"
        import docgen.docgen as m
        assert m.ts_has_missing_docstrings(source) is True

    def test_function_with_jsdoc_returns_false(self):
        source = "/**\n * Greets a user.\n */\nfunction greet(name: string): string {\n  return name;\n}\n"
        import docgen.docgen as m
        assert m.ts_has_missing_docstrings(source) is False

    def test_class_without_jsdoc_returns_true(self):
        source = "class MyService {\n  run() {}\n}\n"
        import docgen.docgen as m
        assert m.ts_has_missing_docstrings(source) is True

    def test_class_with_jsdoc_returns_false(self):
        source = "/**\n * MyService class.\n */\nclass MyService {\n  /** Run it. */\n  run() {}\n}\n"
        import docgen.docgen as m
        assert m.ts_has_missing_docstrings(source) is False

    def test_exported_function_without_jsdoc_returns_true(self):
        source = "export function add(a: number, b: number): number {\n  return a + b;\n}\n"
        import docgen.docgen as m
        assert m.ts_has_missing_docstrings(source) is True

    def test_force_returns_true_even_when_documented(self):
        source = "/**\n * Greets.\n */\nfunction greet() {}\n"
        import docgen.docgen as m
        assert m.ts_has_missing_docstrings(source, force=True) is True

    def test_no_declarations_returns_false(self):
        source = "const x = 1;\nconst y = 2;\n"
        import docgen.docgen as m
        assert m.ts_has_missing_docstrings(source) is False

    def test_class_method_without_jsdoc_returns_true(self):
        source = "/**\n * MyService.\n */\nclass MyService {\n  run(): void {}\n}\n"
        import docgen.docgen as m
        assert m.ts_has_missing_docstrings(source) is True


class TestNeedsDocstrings:
    def test_routes_py_to_python_parser(self):
        with tempfile.TemporaryDirectory() as d:
            f = Path(d) / "foo.py"
            f.write_text("def bar():\n    return 1\n")
            import docgen.docgen as m
            assert m.needs_docstrings(f) is True

    def test_routes_ts_to_ts_parser(self):
        with tempfile.TemporaryDirectory() as d:
            f = Path(d) / "foo.ts"
            f.write_text("function greet() {}\n")
            import docgen.docgen as m
            assert m.needs_docstrings(f) is True

    def test_unknown_extension_returns_false(self):
        with tempfile.TemporaryDirectory() as d:
            f = Path(d) / "foo.rb"
            f.write_text("def bar; end\n")
            import docgen.docgen as m
            assert m.needs_docstrings(f) is False

    def test_returns_false_on_os_error(self):
        import docgen.docgen as m
        assert m.needs_docstrings(Path("/nonexistent/path/foo.py")) is False


class TestGetFormat:
    def test_py_defaults_to_mkdocs(self):
        import docgen.docgen as m
        assert m.get_format(Path("foo.py"), None) == "mkdocs"

    def test_ts_defaults_to_tsdoc(self):
        import docgen.docgen as m
        assert m.get_format(Path("foo.ts"), None) == "tsdoc"

    def test_tsx_defaults_to_tsdoc(self):
        import docgen.docgen as m
        assert m.get_format(Path("foo.tsx"), None) == "tsdoc"

    def test_explicit_format_overrides(self):
        import docgen.docgen as m
        assert m.get_format(Path("foo.py"), "tsdoc") == "tsdoc"


class TestGenerateDocstrings:
    def test_returns_patched_source_on_success(self):
        with tempfile.TemporaryDirectory() as d:
            f = Path(d) / "foo.py"
            f.write_text("def bar():\n    return 1\n")

            mock_client = MagicMock()
            mock_client.chat = MagicMock()
            mock_client.chat.completions = MagicMock()
            mock_client.chat.completions.create = AsyncMock(return_value=MagicMock(
                choices=[MagicMock(message=MagicMock(content='def bar():\n    """Returns 1."""\n    return 1\n'))]
            ))

            import docgen.docgen as m
            sem = asyncio.Semaphore(4)
            result_path, result_content = asyncio.run(
                m.generate_docstrings_async(mock_client, sem, f, "mkdocs", False, "test-model")
            )
        assert result_path == f
        assert '"""Returns 1."""' in result_content

    def test_returns_none_on_llm_error(self):
        with tempfile.TemporaryDirectory() as d:
            f = Path(d) / "foo.py"
            f.write_text("def bar():\n    return 1\n")

            mock_client = MagicMock()
            mock_client.chat.completions.create = AsyncMock(side_effect=Exception("LLM down"))

            import docgen.docgen as m
            sem = asyncio.Semaphore(4)
            result_path, result_content = asyncio.run(
                m.generate_docstrings_async(mock_client, sem, f, "mkdocs", False, "test-model")
            )
        assert result_path == f
        assert result_content is None

    def test_force_true_uses_replace_action_in_prompt(self):
        with tempfile.TemporaryDirectory() as d:
            f = Path(d) / "foo.py"
            f.write_text('def bar():\n    """Already documented."""\n    return 1\n')

            captured_prompt = {}

            async def fake_create(**kwargs):
                captured_prompt["content"] = kwargs["messages"][0]["content"]
                return MagicMock(
                    choices=[MagicMock(message=MagicMock(content="patched"))]
                )

            mock_client = MagicMock()
            mock_client.chat.completions.create = fake_create

            import docgen.docgen as m
            sem = asyncio.Semaphore(4)
            asyncio.run(
                m.generate_docstrings_async(mock_client, sem, f, "mkdocs", True, "test-model")
            )
        assert "Replace all existing docstrings" in captured_prompt["content"]


class TestProcessFilesAsync:
    def test_returns_dict_of_patched_files(self):
        with tempfile.TemporaryDirectory() as d:
            f1 = Path(d) / "a.py"
            f2 = Path(d) / "b.py"
            f1.write_text("def a(): pass")
            f2.write_text("def b(): pass")

            patched_a = 'def a():\n    """Does a."""\n    pass'
            patched_b = 'def b():\n    """Does b."""\n    pass'

            async def fake_generate(client, sem, file, fmt, force, model):
                return (file, patched_a if file.name == "a.py" else patched_b)

            import docgen.docgen as m
            with patch.object(m, "generate_docstrings_async", side_effect=fake_generate):
                result = asyncio.run(
                    m.process_files_async([f1, f2], None, False, "test-model", 4, "http://x/v1")
                )
        assert result[f1] == patched_a
        assert result[f2] == patched_b

    def test_skips_files_where_llm_returns_none(self):
        with tempfile.TemporaryDirectory() as d:
            f1 = Path(d) / "a.py"
            f1.write_text("def a(): pass")

            async def fake_generate(client, sem, file, fmt, force, model):
                return (file, None)

            import docgen.docgen as m
            with patch.object(m, "generate_docstrings_async", side_effect=fake_generate):
                result = asyncio.run(
                    m.process_files_async([f1], None, False, "test-model", 4, "http://x/v1")
                )
        assert result == {}


class TestApplyWithGit:
    def test_creates_branch_writes_files_merges_and_deletes(self):
        with tempfile.TemporaryDirectory() as d:
            f = Path(d) / "foo.py"
            patched = {f: 'def foo():\n    """Docs."""\n    pass\n'}

            mock_repo = MagicMock()
            mock_repo.active_branch.name = "main"

            with patch("docgen.docgen.git.Repo", return_value=mock_repo):
                import docgen.docgen as m
                m.apply_with_git(patched, Path(d))

            # branch created
            branch_call = mock_repo.git.checkout.call_args_list[0]
            assert branch_call.args[0] == "-b"
            assert branch_call.args[1].startswith("docgen/")
            # commit made
            mock_repo.git.commit.assert_called_once()
            # merged back
            mock_repo.git.merge.assert_called_once()
            # branch deleted
            mock_repo.git.branch.assert_called_with("-d", branch_call.args[1])
            # file written
            assert f.read_text() == patched[f]

    def test_leaves_branch_on_merge_failure(self):
        with tempfile.TemporaryDirectory() as d:
            f = Path(d) / "foo.py"
            f.write_text("")
            patched = {f: "patched"}

            mock_repo = MagicMock()
            mock_repo.active_branch.name = "feature/x"
            mock_repo.git.merge.side_effect = Exception("conflict")

            with patch("docgen.docgen.git.Repo", return_value=mock_repo):
                import docgen.docgen as m
                with pytest.raises(Exception, match="conflict"):
                    m.apply_with_git(patched, Path(d))

            # checkout back to original branch called even on failure
            checkout_calls = [c.args for c in mock_repo.git.checkout.call_args_list]
            assert any("feature/x" in str(c) for c in checkout_calls)
            # assert branch was NOT deleted on failure (left for inspection)
            delete_calls = [str(c.args) for c in mock_repo.git.branch.call_args_list]
            assert not any("-d" in call for call in delete_calls)


class TestCli:
    def _runner(self):
        import docgen.docgen as m
        return CliRunner(), m.app

    def test_exits_1_on_dirty_tree(self):
        runner, app = self._runner()
        with patch("docgen.docgen.check_dirty_tree", return_value=["src/foo.py"]):
            result = runner.invoke(app, ["src/"])
        assert result.exit_code == 1
        assert "uncommitted changes" in result.output

    def test_exits_1_when_vllm_unreachable(self):
        runner, app = self._runner()
        with patch("docgen.docgen.check_dirty_tree", return_value=[]), \
             patch("docgen.docgen.check_vllm_reachable", return_value=False):
            result = runner.invoke(app, ["src/"])
        assert result.exit_code == 1
        assert "not reachable" in result.output

    def test_exits_1_when_model_not_detected(self):
        runner, app = self._runner()
        with patch("docgen.docgen.check_dirty_tree", return_value=[]), \
             patch("docgen.docgen.check_vllm_reachable", return_value=True), \
             patch("docgen.docgen.detect_model", return_value=None):
            result = runner.invoke(app, ["src/"])
        assert result.exit_code == 1
        assert "model" in result.output.lower()

    def test_exits_2_when_no_files_found(self):
        runner, app = self._runner()
        with tempfile.TemporaryDirectory() as d:
            with patch("docgen.docgen.check_dirty_tree", return_value=[]), \
                 patch("docgen.docgen.check_vllm_reachable", return_value=True), \
                 patch("docgen.docgen.detect_model", return_value="test-model"):
                result = runner.invoke(app, [d])
        assert result.exit_code == 2
        assert "No Python or TypeScript" in result.output

    def test_exits_2_when_all_files_already_documented(self):
        runner, app = self._runner()
        with tempfile.TemporaryDirectory() as d:
            f = Path(d) / "foo.py"
            f.write_text('def foo():\n    """Already documented."""\n    pass\n')
            with patch("docgen.docgen.check_dirty_tree", return_value=[]), \
                 patch("docgen.docgen.check_vllm_reachable", return_value=True), \
                 patch("docgen.docgen.detect_model", return_value="test-model"):
                result = runner.invoke(app, [str(f)])
        assert result.exit_code == 2

    def test_exits_0_on_success(self):
        runner, app = self._runner()
        with tempfile.TemporaryDirectory() as d:
            f = Path(d) / "foo.py"
            f.write_text("def foo():\n    return 1\n")
            with patch("docgen.docgen.check_dirty_tree", return_value=[]), \
                 patch("docgen.docgen.check_vllm_reachable", return_value=True), \
                 patch("docgen.docgen.detect_model", return_value="test-model"), \
                 patch("docgen.docgen.process_files_async", new_callable=AsyncMock,
                       return_value={f: 'def foo():\n    """Docs."""\n    return 1\n'}), \
                 patch("docgen.docgen.apply_with_git"):
                result = runner.invoke(app, [str(f)])
        assert result.exit_code == 0
