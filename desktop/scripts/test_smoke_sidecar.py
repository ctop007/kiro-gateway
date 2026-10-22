"""
Tests for the sidecar build tooling in desktop/scripts/sidecar/:

- build_helpers.py : production requirements filtering, PyInstaller argument
                     assembly from hidden-imports.txt, tiktoken cache naming.
- smoke_sidecar.py : the post-build smoke check of the frozen gateway.

The smoke tests build small fake "sidecar" executables (Python scripts with a
shebang) whose behavior is baked into their source, then run the smoke check
against them. The fakes stand in for the real frozen gateway so every failure
branch of the smoke check can be exercised without running PyInstaller.

Run with:
    python3 -I -m unittest discover -s desktop/scripts -p 'test_*.py' -v
(-I is fine: the test file inserts the sidecar directory into sys.path.)
"""

import ast
import hashlib
import json
import os
import socket
import sqlite3
import stat
import sys
import tempfile
import textwrap
import unittest
from datetime import datetime, timezone
from pathlib import Path

SCRIPTS_DIR = Path(__file__).resolve().parent
SIDECAR_DIR = SCRIPTS_DIR / "sidecar"
REPO_ROOT = SCRIPTS_DIR.parent.parent
sys.path.insert(0, str(SIDECAR_DIR))

import build_helpers  # noqa: E402  (path set up above)
import smoke_sidecar  # noqa: E402


# --------------------------------------------------------------------------- #
# build_helpers
# --------------------------------------------------------------------------- #


class TestProdRequirements(unittest.TestCase):
    """Only production dependencies of requirements.txt reach the build venv."""

    SAMPLE = textwrap.dedent(
        """\
        # Prod dependencies
        fastapi
        uvicorn[standard]

        httpx  # inline comment
        # Testing dependencies
        pytest
        hypothesis
        """
    )

    def test_stops_at_testing_marker(self) -> None:
        """
        What it does: filters a requirements file that has a testing section.
        Purpose: pytest and hypothesis must never be installed into the sidecar venv.
        """
        self.assertEqual(
            build_helpers.prod_requirements(self.SAMPLE),
            ["fastapi", "uvicorn[standard]", "httpx"],
        )

    def test_without_marker_keeps_everything(self) -> None:
        """A file without a testing section is all production."""
        self.assertEqual(build_helpers.prod_requirements("a\nb\n"), ["a", "b"])

    def test_empty_result_is_an_error(self) -> None:
        """An empty production list means the file layout changed; fail loudly."""
        with self.assertRaises(build_helpers.BuildSpecError):
            build_helpers.prod_requirements("# Testing dependencies\npytest\n")

    def test_marker_match_is_case_insensitive(self) -> None:
        self.assertEqual(
            build_helpers.prod_requirements("a\n# TESTING deps\nb\n"), ["a"]
        )

    def test_real_requirements_file_has_gateway_runtime_deps(self) -> None:
        """
        What it does: reads the repo's real requirements.txt.
        Purpose: the six production packages are present, no test packages leak in.
        """
        text = (REPO_ROOT / "requirements.txt").read_text(encoding="utf-8")
        reqs = build_helpers.prod_requirements(text)
        names = {r.split("[")[0].split("=")[0].strip().lower() for r in reqs}
        self.assertEqual(
            names, {"fastapi", "uvicorn", "httpx", "loguru", "python-dotenv", "tiktoken"}
        )


class TestHiddenImportSpec(unittest.TestCase):
    """hidden-imports.txt is parsed into PyInstaller arguments."""

    def test_parse_skips_comments_and_blanks(self) -> None:
        spec = "# c\n\ncollect-submodules uvicorn\nhidden-import main  # why\n"
        self.assertEqual(
            build_helpers.parse_spec(spec),
            [("collect-submodules", "uvicorn"), ("hidden-import", "main")],
        )

    def test_unknown_kind_is_rejected(self) -> None:
        with self.assertRaises(build_helpers.BuildSpecError) as ctx:
            build_helpers.parse_spec("collect-everything uvicorn\n")
        self.assertIn("collect-everything", str(ctx.exception))

    def test_line_without_value_is_rejected(self) -> None:
        with self.assertRaises(build_helpers.BuildSpecError):
            build_helpers.parse_spec("hidden-import\n")

    def test_line_with_extra_tokens_is_rejected(self) -> None:
        with self.assertRaises(build_helpers.BuildSpecError):
            build_helpers.parse_spec("hidden-import a b\n")

    def test_pyinstaller_args_shape(self) -> None:
        args = build_helpers.pyinstaller_args(
            [("collect-submodules", "uvicorn"), ("hidden-import", "main")]
        )
        self.assertEqual(
            args, ["--collect-submodules", "uvicorn", "--hidden-import", "main"]
        )

    def test_omit_removes_exactly_one_entry(self) -> None:
        """
        What it does: omits one entry from the spec.
        Purpose: the negative check must be able to drop a single hidden import.
        """
        spec = [("collect-submodules", "uvicorn"), ("hidden-import", "main")]
        self.assertEqual(
            build_helpers.pyinstaller_args(spec, omit="main"),
            ["--collect-submodules", "uvicorn"],
        )

    def test_omit_unknown_name_is_an_error(self) -> None:
        """A typo in --omit must not silently pass the negative check."""
        with self.assertRaises(build_helpers.BuildSpecError) as ctx:
            build_helpers.pyinstaller_args([("hidden-import", "main")], omit="mian")
        self.assertIn("mian", str(ctx.exception))

    def test_real_spec_parses_and_covers_the_known_dynamic_imports(self) -> None:
        """
        What it does: parses the real hidden-imports.txt.
        Purpose: uvicorn's string-based plugin imports, the 'main:app' string
        import and tiktoken's namespace plugin are all declared.
        """
        spec = build_helpers.parse_spec(
            (SIDECAR_DIR / "hidden-imports.txt").read_text(encoding="utf-8")
        )
        self.assertIn(("collect-submodules", "uvicorn"), spec)
        self.assertIn(("collect-submodules", "kiro"), spec)
        self.assertIn(("hidden-import", "main"), spec)
        self.assertIn(("hidden-import", "tiktoken_ext.openai_public"), spec)

    def test_every_function_level_kiro_import_is_collected(self) -> None:
        """
        What it does: walks kiro/*.py with ast and finds imports of kiro.* inside
        function bodies (the ones that are easy to miss in a frozen build).
        Purpose: each such module must be covered by a collect-submodules entry.
        """
        spec = build_helpers.parse_spec(
            (SIDECAR_DIR / "hidden-imports.txt").read_text(encoding="utf-8")
        )
        collected = {v for k, v in spec if k == "collect-submodules"}
        hidden = {v for k, v in spec if k == "hidden-import"}
        found = set()
        for path in (REPO_ROOT / "kiro").glob("*.py"):
            tree = ast.parse(path.read_text(encoding="utf-8"))
            for func in ast.walk(tree):
                if not isinstance(func, (ast.FunctionDef, ast.AsyncFunctionDef)):
                    continue
                for node in ast.walk(func):
                    if isinstance(node, ast.ImportFrom) and node.module:
                        if node.module == "kiro" or node.module.startswith("kiro."):
                            found.add(node.module)
        self.assertTrue(found, "expected to find function-level kiro imports")
        for module in sorted(found):
            covered = module in hidden or any(
                module == c or module.startswith(c + ".") for c in collected
            )
            self.assertTrue(covered, f"{module} is imported inside a function but not collected")


class TestCommandLine(unittest.TestCase):
    """The CLI the build scripts call; stdout is what the scripts consume."""

    def _run(self, *argv: str) -> "tuple[int, str, str]":
        import contextlib
        import io

        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = build_helpers._main(list(argv))
        return code, out.getvalue(), err.getvalue()

    def test_requirements_prints_one_per_line(self) -> None:
        code, out, _ = self._run("requirements", str(REPO_ROOT / "requirements.txt"))
        self.assertEqual(code, 0)
        self.assertIn("uvicorn[standard]", out.splitlines())
        self.assertNotIn("pytest", out)

    def test_requirements_missing_file_reports_error(self) -> None:
        code, _, err = self._run("requirements", "/nonexistent/requirements.txt")
        self.assertEqual(code, 1)
        self.assertIn("错误", err)

    def test_pinned_versions_are_printed(self) -> None:
        self.assertEqual(self._run("pyinstaller-version")[1].strip(), build_helpers.PYINSTALLER_VERSION)
        self.assertEqual(self._run("min-python")[1].strip(), "3.10")

    def test_pyinstaller_args_with_and_without_omit(self) -> None:
        spec = str(SIDECAR_DIR / "hidden-imports.txt")
        code, out, _ = self._run("pyinstaller-args", spec)
        self.assertEqual(code, 0)
        lines = out.splitlines()
        self.assertIn("--hidden-import", lines)
        self.assertIn("main", lines)
        code, out, _ = self._run("pyinstaller-args", spec, "--omit", "main")
        self.assertEqual(code, 0)
        self.assertNotIn("main", out.splitlines())

    def test_pyinstaller_args_bad_omit_fails(self) -> None:
        code, _, err = self._run(
            "pyinstaller-args", str(SIDECAR_DIR / "hidden-imports.txt"), "--omit", "mian"
        )
        self.assertEqual(code, 1)
        self.assertIn("mian", err)

    def test_pyinstaller_version_is_pinned_exactly(self) -> None:
        """A range or empty pin would make the build irreproducible."""
        self.assertRegex(build_helpers.PYINSTALLER_VERSION, r"^\d+\.\d+\.\d+$")


class TestDownloadCl100k(unittest.TestCase):
    """download_cl100k verifies the hash and never leaves a partial file."""

    def _patch_urlopen(self, payload: bytes):
        import io
        from unittest import mock

        class _Resp(io.BytesIO):
            def __enter__(self):
                return self

            def __exit__(self, *exc):
                return False

        return mock.patch.object(
            build_helpers.urllib.request, "urlopen", lambda *a, **k: _Resp(payload)
        )

    def test_hash_mismatch_is_rejected_and_nothing_is_written(self) -> None:
        with tempfile.TemporaryDirectory() as tmp, self._patch_urlopen(b"not the real file"):
            with self.assertRaises(build_helpers.BuildSpecError):
                build_helpers.download_cl100k(Path(tmp) / "cache")
            self.assertEqual(list((Path(tmp) / "cache").iterdir()), [])

    def test_network_failure_is_reported_actionably(self) -> None:
        from unittest import mock

        with tempfile.TemporaryDirectory() as tmp, mock.patch.object(
            build_helpers.urllib.request, "urlopen", side_effect=OSError("boom")
        ):
            with self.assertRaises(build_helpers.BuildSpecError) as ctx:
                build_helpers.download_cl100k(Path(tmp))
        self.assertIn("检查网络", str(ctx.exception))

    def test_valid_existing_file_is_reused_without_network(self) -> None:
        """A cached file that matches the pinned sha256 must not trigger a download."""
        from unittest import mock

        with tempfile.TemporaryDirectory() as tmp:
            payload = b"pretend bpe"
            target = Path(tmp) / build_helpers.cl100k_cache_file_name()
            target.write_bytes(payload)
            with mock.patch.object(build_helpers, "CL100K_SHA256", hashlib.sha256(payload).hexdigest()), \
                    mock.patch.object(build_helpers.urllib.request, "urlopen", side_effect=AssertionError("network used")):
                self.assertEqual(build_helpers.download_cl100k(Path(tmp)), target)

    def test_corrupt_existing_file_is_replaced(self) -> None:
        from unittest import mock

        payload = b"pretend bpe 2"
        with tempfile.TemporaryDirectory() as tmp:
            target = Path(tmp) / build_helpers.cl100k_cache_file_name()
            target.write_bytes(b"corrupt")
            with mock.patch.object(build_helpers, "CL100K_SHA256", hashlib.sha256(payload).hexdigest()), \
                    self._patch_urlopen(payload):
                build_helpers.download_cl100k(Path(tmp))
            self.assertEqual(target.read_bytes(), payload)
            self.assertFalse(list(Path(tmp).glob("*.part")))


class TestTiktokenCacheName(unittest.TestCase):
    """The bundled BPE file must be named exactly like tiktoken looks it up."""

    def test_name_is_sha1_of_url(self) -> None:
        url = "https://openaipublic.blob.core.windows.net/encodings/cl100k_base.tiktoken"
        self.assertEqual(build_helpers.CL100K_URL, url)
        self.assertEqual(
            build_helpers.cl100k_cache_file_name(), hashlib.sha1(url.encode()).hexdigest()
        )

    def test_url_matches_installed_tiktoken(self) -> None:
        """
        What it does: reads cl100k_base's URL from the installed tiktoken_ext.
        Purpose: if tiktoken ever changes the blob URL, the cache name goes stale
        and the sidecar would try to download at runtime; catch it here.
        """
        try:
            import tiktoken_ext.openai_public as public  # type: ignore
        except ImportError:
            self.skipTest("tiktoken is not installed in this interpreter")
        source = Path(public.__file__).read_text(encoding="utf-8")
        self.assertIn(build_helpers.CL100K_URL, source)


# --------------------------------------------------------------------------- #
# smoke_sidecar
# --------------------------------------------------------------------------- #

# Fake sidecar template. Placeholders are replaced per test:
#   HEALTH_STATUS  : int, status of /health
#   MODELS_STATUS  : int, status of /v1/models
#   COUNT_STATUS   : int, status of /v1/messages/count_tokens
#   TOKENIZER_LOG  : log line printed after count_tokens
#   HANG           : bool, never bind the port
#   CRASH          : bool, crash at startup like a missing hidden import
#   ENV_DUMP       : path to dump the received environment to ("" = no dump)
FAKE_TEMPLATE = textwrap.dedent(
    '''
    import json, os, sqlite3, sys, time
    from http.server import BaseHTTPRequestHandler, HTTPServer

    HEALTH_STATUS = {health_status!r}
    MODELS_STATUS = {models_status!r}
    COUNT_STATUS = {count_status!r}
    TOKENIZER_LOG = {tokenizer_log!r}
    HANG = {hang!r}
    CRASH = {crash!r}
    ENV_DUMP = {env_dump!r}

    if ENV_DUMP:
        with open(ENV_DUMP, "w") as f:
            json.dump({{"env": dict(os.environ), "cwd": os.getcwd()}}, f)

    if CRASH:
        print("ModuleNotFoundError: No module named 'main'", file=sys.stderr, flush=True)
        sys.exit(1)

    db = os.environ.get("KIRO_CLI_DB_FILE", "")
    if not db or not os.path.exists(db):
        print("ERROR |   No Kiro credentials configured!", file=sys.stderr, flush=True)
        sys.exit(1)
    conn = sqlite3.connect(db)
    row = conn.execute("SELECT value FROM auth_kv WHERE key = 'kirocli:odic:token'").fetchone()
    conn.close()
    if row is None:
        print("ERROR | token row missing", file=sys.stderr, flush=True)
        sys.exit(1)

    if HANG:
        time.sleep(3600)

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def _send(self, status, body):
            data = json.dumps(body).encode()
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def do_GET(self):
            if self.path == "/health":
                self._send(HEALTH_STATUS, {{"status": "healthy"}})
            elif self.path == "/v1/models":
                if self.headers.get("Authorization") != "Bearer test":
                    self._send(401, {{"detail": "bad key"}})
                else:
                    self._send(MODELS_STATUS, {{"object": "list", "data": [{{"id": "claude-sonnet-4.5"}}]}})
            else:
                self._send(404, {{}})

        def do_POST(self):
            length = int(self.headers.get("Content-Length", "0"))
            self.rfile.read(length)
            if self.path == "/v1/messages/count_tokens" and self.headers.get("x-api-key") == "test":
                print(TOKENIZER_LOG, file=sys.stderr, flush=True)
                self._send(COUNT_STATUS, {{"input_tokens": 12}})
            else:
                self._send(404, {{}})

    HTTPServer((os.environ["SERVER_HOST"], int(os.environ["SERVER_PORT"])), Handler).serve_forever()
    '''
)

GOOD_TOKENIZER_LOG = "DEBUG | [Tokenizer] Initialized tiktoken with cl100k_base encoding"
BAD_TOKENIZER_LOG = (
    "ERROR | [Tokenizer] Failed to initialize tiktoken (will retry on next call): "
    "Unknown encoding cl100k_base"
)


def make_fake(
    directory: Path,
    health_status: int = 200,
    models_status: int = 200,
    count_status: int = 200,
    tokenizer_log: str = GOOD_TOKENIZER_LOG,
    hang: bool = False,
    crash: bool = False,
    env_dump: str = "",
) -> Path:
    """
    Write an executable fake sidecar into ``directory``.

    Args:
        directory: Directory to place the fake in.
        health_status: HTTP status returned by /health.
        models_status: HTTP status returned by /v1/models.
        count_status: HTTP status returned by count_tokens.
        tokenizer_log: Line logged when count_tokens is served.
        hang: Never open the port when True.
        crash: Exit at startup like a frozen app missing a hidden import.
        env_dump: Path to dump the received environment to.

    Returns:
        Path to the executable fake.
    """
    path = directory / "fake-sidecar"
    source = FAKE_TEMPLATE.format(
        health_status=health_status,
        models_status=models_status,
        count_status=count_status,
        tokenizer_log=tokenizer_log,
        hang=hang,
        crash=crash,
        env_dump=env_dump,
    )
    path.write_text(f"#!{sys.executable}\n{source}", encoding="utf-8")
    path.chmod(path.stat().st_mode | stat.S_IXUSR)
    return path


def make_cache(directory: Path) -> Path:
    """Create a tiktoken cache directory holding a plausible cl100k_base file."""
    cache = directory / "tiktoken_cache"
    cache.mkdir()
    (cache / build_helpers.cl100k_cache_file_name()).write_bytes(b"x" * 2_000_000)
    return cache


@unittest.skipIf(os.name == "nt", "fake sidecars rely on shebang scripts")
class TestRunSmokeSuccess(unittest.TestCase):
    """The smoke check passes for a sidecar that behaves like the real gateway."""

    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.tmp = Path(self._tmp.name)
        self.cache = make_cache(self.tmp)

    def tearDown(self) -> None:
        self._tmp.cleanup()

    def test_well_behaved_sidecar_passes(self) -> None:
        """
        What it does: runs the smoke check against a correct fake.
        Purpose: the happy path must return without raising.
        """
        smoke_sidecar.run_smoke(make_fake(self.tmp), self.cache, startup_timeout=15.0)

    def test_sidecar_gets_isolated_environment(self) -> None:
        """
        What it does: dumps the environment and cwd the fake receives.
        Purpose: HOME/cwd are fresh temp dirs, inherited ANTHROPIC_*/PROXY_*/KIRO_*
        are dropped, the network is pointed at a dead proxy, config matches design
        decision 2 for the variables the smoke check controls.
        """
        dump = self.tmp / "env.json"
        exe = make_fake(self.tmp, env_dump=str(dump))
        leaked = {
            "ANTHROPIC_BASE_URL": "http://leak.invalid",
            "PROXY_API_KEY": "leaked-key",
            "KIRO_CREDS_FILE": "/leaked/creds.json",
            "REFRESH_TOKEN": "leaked-refresh",
            "HTTPS_PROXY": "http://leak.invalid:3128",
        }
        saved = {k: os.environ.get(k) for k in leaked}
        os.environ.update(leaked)
        try:
            smoke_sidecar.run_smoke(exe, self.cache, startup_timeout=15.0)
        finally:
            for key, old in saved.items():
                if old is None:
                    os.environ.pop(key, None)
                else:
                    os.environ[key] = old

        data = json.loads(dump.read_text())
        env, cwd = data["env"], data["cwd"]
        for key in ("ANTHROPIC_BASE_URL", "KIRO_CREDS_FILE", "REFRESH_TOKEN"):
            self.assertNotIn(key, env)
        self.assertEqual(env["PROXY_API_KEY"], "test")
        self.assertEqual(env["SERVER_HOST"], "127.0.0.1")
        self.assertTrue(8100 <= int(env["SERVER_PORT"]) <= 8199)
        self.assertEqual(env["TIKTOKEN_CACHE_DIR"], str(self.cache))
        self.assertEqual(env["LOG_LEVEL"], "DEBUG")
        self.assertEqual(env["FAKE_REASONING"], "false")
        # The startup banner prints emoji; a redirected stdout on Windows would
        # use the ANSI code page and crash without these.
        self.assertEqual(env["PYTHONUTF8"], "1")
        self.assertEqual(env["PYTHONIOENCODING"], "utf-8")
        for key in ("HTTPS_PROXY", "HTTP_PROXY", "ALL_PROXY"):
            self.assertEqual(env[key], smoke_sidecar.DEAD_PROXY)
        self.assertIn("127.0.0.1", env["NO_PROXY"])
        self.assertNotEqual(env["HOME"], str(Path.home()))
        self.assertTrue(env["KIRO_CLI_DB_FILE"].startswith(env["HOME"]) or "kgw-smoke" in env["KIRO_CLI_DB_FILE"])
        self.assertNotEqual(os.path.realpath(cwd), os.path.realpath(os.getcwd()))

    def test_temp_dirs_are_removed(self) -> None:
        """No temp HOME/cwd may be left behind after a run."""
        dump = self.tmp / "env.json"
        smoke_sidecar.run_smoke(
            make_fake(self.tmp, env_dump=str(dump)), self.cache, startup_timeout=15.0
        )
        data = json.loads(dump.read_text())
        self.assertFalse(os.path.exists(data["env"]["HOME"]))
        self.assertFalse(os.path.exists(data["cwd"]))


@unittest.skipIf(os.name == "nt", "fake sidecars rely on shebang scripts")
class TestRunSmokeFailures(unittest.TestCase):
    """Every way the frozen sidecar can be broken must fail the smoke check."""

    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.tmp = Path(self._tmp.name)
        self.cache = make_cache(self.tmp)

    def tearDown(self) -> None:
        self._tmp.cleanup()

    def _assert_fails(self, exe: Path, fragment: str, timeout: float = 15.0) -> None:
        with self.assertRaises(smoke_sidecar.SmokeError) as ctx:
            smoke_sidecar.run_smoke(exe, self.cache, startup_timeout=timeout)
        self.assertIn(fragment, str(ctx.exception))

    def test_crash_at_startup_fails_and_surfaces_output(self) -> None:
        """A frozen app missing a hidden import crashes; the crash output must surface."""
        self._assert_fails(make_fake(self.tmp, crash=True), "No module named 'main'")

    def test_health_not_200_fails(self) -> None:
        """/health answering 503 forever must time out as a failure."""
        self._assert_fails(make_fake(self.tmp, health_status=503), "/health", timeout=3.0)

    def test_never_listening_fails(self) -> None:
        """A sidecar that never opens its port must time out."""
        self._assert_fails(make_fake(self.tmp, hang=True), "/health", timeout=3.0)

    def test_models_endpoint_error_fails(self) -> None:
        """/v1/models must return 200 with a non-empty model list."""
        self._assert_fails(make_fake(self.tmp, models_status=500), "/v1/models")

    def test_count_tokens_error_fails(self) -> None:
        """count_tokens failing means the tokenizer path is broken."""
        self._assert_fails(make_fake(self.tmp, count_status=500), "count_tokens")

    def test_tokenizer_fallback_fails(self) -> None:
        """tiktoken silently falling back (missing tiktoken_ext) must be caught."""
        self._assert_fails(make_fake(self.tmp, tokenizer_log=BAD_TOKENIZER_LOG), "tiktoken")

    def test_tokenizer_never_logging_init_fails(self) -> None:
        """No 'Initialized tiktoken' line means the encoding was never loaded."""
        self._assert_fails(make_fake(self.tmp, tokenizer_log="INFO | nothing"), "tiktoken", timeout=3.0)

    def test_missing_executable_fails(self) -> None:
        """A missing build output is reported, not a raw FileNotFoundError."""
        self._assert_fails(self.tmp / "does-not-exist", "找不到")

    def test_missing_tiktoken_cache_fails(self) -> None:
        """The BPE cache directory must exist and hold the cl100k_base file."""
        with self.assertRaises(smoke_sidecar.SmokeError) as ctx:
            smoke_sidecar.check_tiktoken_cache(self.tmp / "nope")
        self.assertIn("tiktoken", str(ctx.exception))
        empty = self.tmp / "empty"
        empty.mkdir()
        with self.assertRaises(smoke_sidecar.SmokeError):
            smoke_sidecar.check_tiktoken_cache(empty)

    def test_truncated_tiktoken_cache_fails(self) -> None:
        """A partial download must not pass as a valid cache."""
        (self.cache / build_helpers.cl100k_cache_file_name()).write_bytes(b"x" * 10)
        with self.assertRaises(smoke_sidecar.SmokeError):
            smoke_sidecar.check_tiktoken_cache(self.cache)

    def test_misnamed_tiktoken_cache_fails(self) -> None:
        """A file that is not named sha1(url) would be ignored by tiktoken."""
        (self.cache / build_helpers.cl100k_cache_file_name()).rename(self.cache / "cl100k_base.tiktoken")
        with self.assertRaises(smoke_sidecar.SmokeError):
            smoke_sidecar.check_tiktoken_cache(self.cache)

    def test_sidecar_is_stopped_after_failure(self) -> None:
        """No sidecar process may outlive a failed smoke run."""
        exe = make_fake(self.tmp, models_status=500)
        with self.assertRaises(smoke_sidecar.SmokeError):
            smoke_sidecar.run_smoke(exe, self.cache, startup_timeout=15.0)
        self.assertTrue(smoke_sidecar.STARTED_PROCESSES)
        for proc in smoke_sidecar.STARTED_PROCESSES:
            self.assertIsNotNone(proc.poll())

    def test_sidecar_is_stopped_after_success(self) -> None:
        """No sidecar process may outlive a successful smoke run either."""
        smoke_sidecar.STARTED_PROCESSES.clear()
        smoke_sidecar.run_smoke(make_fake(self.tmp), self.cache, startup_timeout=15.0)
        self.assertEqual(len(smoke_sidecar.STARTED_PROCESSES), 1)
        self.assertIsNotNone(smoke_sidecar.STARTED_PROCESSES[0].poll())


class TestFakeKiroCliDb(unittest.TestCase):
    """The fake kiro-cli database mirrors the real auth_kv layout."""

    def _read(self, key: str) -> dict:
        with tempfile.TemporaryDirectory() as tmp:
            db = Path(tmp) / "data.sqlite3"
            smoke_sidecar.create_fake_kiro_cli_db(db)
            conn = sqlite3.connect(db)
            try:
                columns = [row[1] for row in conn.execute("PRAGMA table_info(auth_kv)")]
                row = conn.execute("SELECT value FROM auth_kv WHERE key = ?", (key,)).fetchone()
            finally:
                conn.close()
        self.assertEqual(columns, ["key", "value"])
        self.assertIsNotNone(row, key)
        return json.loads(row[0])

    def test_token_row_is_oidc_with_far_future_expiry(self) -> None:
        """
        What it does: reads the odic token row back like kiro/auth.py does.
        Purpose: the gateway must find the token under a known key with an
        expires_at far enough ahead that it never tries to refresh (no network).
        """
        token = self._read("kirocli:odic:token")
        self.assertTrue(token["access_token"].startswith("smoke-fake-"))
        self.assertTrue(token["refresh_token"].startswith("smoke-fake-"))
        self.assertEqual(token["region"], "us-east-1")
        expires = datetime.fromisoformat(token["expires_at"].replace("Z", "+00:00"))
        remaining = (expires - datetime.now(timezone.utc)).total_seconds()
        # Gateway refreshes when < 600s remain; a year is far beyond that.
        self.assertGreater(remaining, 364 * 86400)

    def test_device_registration_row_present(self) -> None:
        """client_id and client_secret make the gateway pick AWS SSO OIDC auth."""
        reg = self._read("kirocli:odic:device-registration")
        self.assertTrue(reg["client_id"].startswith("smoke-fake-"))
        self.assertTrue(reg["client_secret"].startswith("smoke-fake-"))


class TestPickFreePort(unittest.TestCase):
    """Port selection stays inside 8100-8199 and skips busy ports."""

    def test_port_in_range_and_bindable(self) -> None:
        port = smoke_sidecar.pick_free_port()
        self.assertTrue(8100 <= port <= 8199)
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
            s.bind(("127.0.0.1", port))

    def test_busy_port_skipped(self) -> None:
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as busy:
            busy.bind(("127.0.0.1", 0))
            busy.listen()
            taken = busy.getsockname()[1]
            free = smoke_sidecar.pick_free_port(candidates=[taken, *range(8100, 8200)])
        self.assertNotEqual(free, taken)

    def test_all_busy_raises(self) -> None:
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as busy:
            busy.bind(("127.0.0.1", 0))
            busy.listen()
            taken = busy.getsockname()[1]
            with self.assertRaises(smoke_sidecar.SmokeError) as ctx:
                smoke_sidecar.pick_free_port(candidates=[taken])
        self.assertIn("端口", str(ctx.exception))


if __name__ == "__main__":
    unittest.main()
