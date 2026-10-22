"""
Post-build smoke check for the frozen gateway sidecar.

Starts the sidecar the way the desktop app will (environment only, no .env),
inside a throw-away HOME and cwd, on a free 81xx port, with a fake kiro-cli
database and a dead network proxy. It then checks that the frozen app really
works, not just that the file exists:

1. /health answers 200 (all imports resolved, lifespan finished).
2. /v1/models answers 200 with a non-empty list (account initialised from the
   fake kiro-cli database without touching the network).
3. /v1/messages/count_tokens answers 200 and the log shows
   "Initialized tiktoken" (BPE file came from TIKTOKEN_CACHE_DIR, tiktoken_ext
   plugin found, no silent fallback to the length heuristic).

Usage:
    python smoke_sidecar.py <sidecar-executable> <tiktoken-cache-dir> [--timeout 60]

Exit code 0 on success, 1 on any failure (with the sidecar output on stderr).
"""

import argparse
import json
import os
import shutil
import signal
import socket
import sqlite3
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Dict, Iterable, List, Optional

sys.path.insert(0, str(Path(__file__).resolve().parent))

from build_helpers import cl100k_cache_file_name  # noqa: E402

PORT_RANGE = range(8100, 8200)
API_KEY = "test"
# Nothing listens on the discard port; any outbound connection fails at once.
DEAD_PROXY = "http://127.0.0.1:9"
MIN_BPE_BYTES = 1_000_000  # real cl100k_base.tiktoken is about 1.7 MB
LOG_TAIL_CHARS = 4000

# Processes started by this module; tests assert none outlives a run.
STARTED_PROCESSES: List["subprocess.Popen[bytes]"] = []

# Variables the sidecar may inherit. Everything else (ANTHROPIC_*, PROXY_*,
# KIRO_*, REFRESH_TOKEN, ...) is dropped on purpose.
_PASSTHROUGH_ENV = (
    "PATH", "LANG", "LC_ALL", "TMPDIR", "TEMP", "TMP",
    "SYSTEMROOT", "COMSPEC", "PATHEXT", "WINDIR",
)


class SmokeError(Exception):
    """Raised when the sidecar fails the smoke check."""


def pick_free_port(candidates: Optional[Iterable[int]] = None) -> int:
    """
    Pick a port in 8100-8199 that can be bound on 127.0.0.1.

    Args:
        candidates: Ports to try in order (default: the whole range, shuffled
            start so parallel runs rarely collide).

    Returns:
        A bindable port.

    Raises:
        SmokeError: If every candidate is busy.
    """
    if candidates is None:
        ports = list(PORT_RANGE)
        offset = os.getpid() % len(ports)
        candidates = ports[offset:] + ports[:offset]
    for port in candidates:
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
            try:
                sock.bind(("127.0.0.1", port))
            except OSError:
                continue
            return port
    raise SmokeError("8100-8199 里没有空闲端口，释放一个端口后重试")


def create_fake_kiro_cli_db(path: Path) -> None:
    """
    Create a kiro-cli style SQLite database with fake OIDC credentials.

    Mirrors the layout kiro/auth.py reads: an ``auth_kv`` table with the token
    under ``kirocli:odic:token`` and the client registration under
    ``kirocli:odic:device-registration``. The token expires one year from now so
    the gateway never tries to refresh it (which would need the network).

    Args:
        path: Database file to create.
    """
    expires = (datetime.now(timezone.utc) + timedelta(days=365)).strftime("%Y-%m-%dT%H:%M:%SZ")
    token = {
        "access_token": "smoke-fake-access-token",
        "refresh_token": "smoke-fake-refresh-token",
        "expires_at": expires,
        "region": "us-east-1",
    }
    registration = {
        "client_id": "smoke-fake-client-id",
        "client_secret": "smoke-fake-client-secret",
        "region": "us-east-1",
    }
    conn = sqlite3.connect(str(path))
    try:
        conn.execute("CREATE TABLE auth_kv (key TEXT PRIMARY KEY, value TEXT)")
        conn.execute("INSERT INTO auth_kv VALUES (?, ?)", ("kirocli:odic:token", json.dumps(token)))
        conn.execute(
            "INSERT INTO auth_kv VALUES (?, ?)",
            ("kirocli:odic:device-registration", json.dumps(registration)),
        )
        conn.commit()
    finally:
        conn.close()


def check_tiktoken_cache(cache_dir: Path) -> Path:
    """
    Verify the bundled tiktoken cache holds cl100k_base under its lookup name.

    Args:
        cache_dir: Directory that will be TIKTOKEN_CACHE_DIR.

    Returns:
        Path of the cache file.

    Raises:
        SmokeError: If the directory or file is missing, misnamed or truncated.
    """
    name = cl100k_cache_file_name()
    target = cache_dir / name
    if not cache_dir.is_dir() or not target.is_file():
        raise SmokeError(
            f"tiktoken 缓存目录里没有 {name}（cl100k_base，文件名必须是下载地址的 sha1），{cache_dir}"
        )
    size = target.stat().st_size
    if size < MIN_BPE_BYTES:
        raise SmokeError(f"tiktoken 缓存文件只 {size} 字节，下载不完整：{target}")
    return target


def build_environment(home: Path, port: int, db_file: Path, cache_dir: Path) -> Dict[str, str]:
    """
    Assemble the sidecar environment (a subset of design decision 2).

    Args:
        home: Throw-away HOME.
        port: Port to listen on.
        db_file: Fake kiro-cli database.
        cache_dir: TIKTOKEN_CACHE_DIR.

    Returns:
        A complete environment mapping; nothing is inherited beyond the
        pass-through list.
    """
    env = {k: os.environ[k] for k in _PASSTHROUGH_ENV if k in os.environ}
    env.update(
        {
            "HOME": str(home),
            "USERPROFILE": str(home),
            "PROXY_API_KEY": API_KEY,
            "KIRO_CLI_DB_FILE": str(db_file),
            "SERVER_HOST": "127.0.0.1",
            "SERVER_PORT": str(port),
            "FAKE_REASONING": "false",
            "TIKTOKEN_CACHE_DIR": str(cache_dir),
            "LOG_LEVEL": "DEBUG",
            "HTTP_PROXY": DEAD_PROXY,
            "HTTPS_PROXY": DEAD_PROXY,
            "ALL_PROXY": DEAD_PROXY,
            "NO_PROXY": "127.0.0.1,localhost",
            "PYTHONUNBUFFERED": "1",
            # The startup banner prints emoji; with stdout redirected to a file a
            # Windows ANSI code page would raise UnicodeEncodeError.
            "PYTHONUTF8": "1",
            "PYTHONIOENCODING": "utf-8",
        }
    )
    return env


def _http(
    method: str,
    url: str,
    headers: Optional[Dict[str, str]] = None,
    body: Optional[dict] = None,
    timeout: float = 5.0,
) -> "tuple[int, bytes]":
    """Send one request to the local sidecar, bypassing any proxy; return (status, body)."""
    data = json.dumps(body).encode() if body is not None else None
    request = urllib.request.Request(url, data=data, method=method)
    for key, value in (headers or {}).items():
        request.add_header(key, value)
    if data is not None:
        request.add_header("Content-Type", "application/json")
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    try:
        with opener.open(request, timeout=timeout) as response:
            return response.status, response.read()
    except urllib.error.HTTPError as exc:
        return exc.code, exc.read()


def _read_log(log_path: Path) -> str:
    try:
        return log_path.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return ""


def _tail(text: str) -> str:
    return text[-LOG_TAIL_CHARS:] if len(text) > LOG_TAIL_CHARS else text


def _fail(message: str, log_path: Path) -> SmokeError:
    return SmokeError(f"{message}\n--- sidecar 输出（末尾）---\n{_tail(_read_log(log_path))}")


def _wait_for_health(proc: "subprocess.Popen[bytes]", port: int, timeout: float, log_path: Path) -> None:
    """Poll /health every 200 ms until 200, a crash, or the timeout."""
    deadline = time.monotonic() + timeout
    url = f"http://127.0.0.1:{port}/health"
    while time.monotonic() < deadline:
        code = proc.poll()
        if code is not None:
            raise _fail(f"sidecar 启动阶段退出了（退出码 {code}）", log_path)
        try:
            status, _ = _http("GET", url, timeout=2.0)
            if status == 200:
                return
        except (urllib.error.URLError, OSError):
            pass
        time.sleep(0.2)
    raise _fail(f"{timeout:.0f} 秒内 /health 没有返回 200", log_path)


def _stop(proc: "subprocess.Popen[bytes]") -> None:
    """Terminate the sidecar (and its process group on POSIX); kill after 5 seconds."""
    if proc.poll() is not None:
        return
    try:
        if os.name == "nt":
            proc.terminate()
        else:
            os.killpg(proc.pid, signal.SIGTERM)
    except (ProcessLookupError, PermissionError):
        pass
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        try:
            if os.name == "nt":
                proc.kill()
            else:
                os.killpg(proc.pid, signal.SIGKILL)
        except (ProcessLookupError, PermissionError):
            pass
        proc.wait(timeout=5)


def _check_endpoints(port: int, log_path: Path, timeout: float) -> None:
    """Run the /v1/models and count_tokens checks plus the tiktoken log assertions."""
    base = f"http://127.0.0.1:{port}"
    status, body = _http("GET", f"{base}/v1/models", {"Authorization": f"Bearer {API_KEY}"})
    if status != 200:
        raise _fail(f"/v1/models 返回 {status}，期望 200", log_path)
    try:
        models = json.loads(body).get("data", [])
    except (ValueError, AttributeError):
        models = []
    if not models:
        raise _fail("/v1/models 返回的模型列表为空", log_path)

    status, body = _http(
        "POST",
        f"{base}/v1/messages/count_tokens",
        {"x-api-key": API_KEY},
        {"model": "claude-sonnet-4.5", "messages": [{"role": "user", "content": "hello smoke test"}]},
    )
    if status != 200:
        raise _fail(f"/v1/messages/count_tokens 返回 {status}，期望 200（tokenizer 路径坏了）", log_path)

    deadline = time.monotonic() + min(timeout, 10.0)
    while True:
        log = _read_log(log_path)
        if "Failed to initialize tiktoken" in log or "tiktoken not installed" in log:
            raise _fail("tiktoken 没有初始化成功，count_tokens 退回了估算（缺 tiktoken_ext 或缓存文件不对）", log_path)
        if "Initialized tiktoken" in log:
            return
        if time.monotonic() >= deadline:
            raise _fail("日志里没有 'Initialized tiktoken'，tiktoken 编码没有被加载", log_path)
        time.sleep(0.2)


def run_smoke(executable: Path, cache_dir: Path, startup_timeout: float = 60.0) -> None:
    """
    Run the full smoke check against a sidecar executable.

    Args:
        executable: Path of the frozen gateway (or a fake in tests).
        cache_dir: Directory used as TIKTOKEN_CACHE_DIR.
        startup_timeout: Seconds to wait for /health.

    Raises:
        SmokeError: On any failed check. The sidecar is always stopped and the
            temporary HOME and cwd are always removed.
    """
    executable = Path(executable)
    if not executable.is_file():
        raise SmokeError(f"找不到 sidecar 可执行文件：{executable}")
    check_tiktoken_cache(Path(cache_dir))

    home = Path(tempfile.mkdtemp(prefix="kgw-smoke-home-"))
    cwd = Path(tempfile.mkdtemp(prefix="kgw-smoke-cwd-"))
    log_dir = Path(tempfile.mkdtemp(prefix="kgw-smoke-log-"))
    log_path = log_dir / "sidecar.log"
    proc: Optional["subprocess.Popen[bytes]"] = None
    try:
        db_file = home / "data.sqlite3"
        create_fake_kiro_cli_db(db_file)
        port = pick_free_port()
        env = build_environment(home, port, db_file, Path(cache_dir))
        with open(log_path, "wb") as log_file:
            try:
                proc = subprocess.Popen(
                    [str(executable)],
                    cwd=str(cwd),
                    env=env,
                    stdin=subprocess.DEVNULL,
                    stdout=log_file,
                    stderr=subprocess.STDOUT,
                    start_new_session=(os.name != "nt"),
                )
            except OSError as exc:
                raise SmokeError(f"启动 sidecar 失败：{exc}") from exc
            STARTED_PROCESSES.append(proc)
            try:
                _wait_for_health(proc, port, startup_timeout, log_path)
                _check_endpoints(port, log_path, startup_timeout)
            finally:
                _stop(proc)
    finally:
        if proc is not None:
            _stop(proc)
        shutil.rmtree(home, ignore_errors=True)
        shutil.rmtree(cwd, ignore_errors=True)
        shutil.rmtree(log_dir, ignore_errors=True)


def main(argv: Optional[List[str]] = None) -> int:
    """CLI entry: returns 0 when the smoke check passes, 1 otherwise."""
    parser = argparse.ArgumentParser(description="Smoke-check a frozen kiro-gateway sidecar")
    parser.add_argument("executable", type=Path)
    parser.add_argument("cache_dir", type=Path)
    parser.add_argument("--timeout", type=float, default=60.0, help="seconds to wait for /health")
    args = parser.parse_args(argv)
    try:
        run_smoke(args.executable, args.cache_dir, startup_timeout=args.timeout)
    except SmokeError as exc:
        print(f"冒烟失败：{exc}", file=sys.stderr)
        return 1
    print("冒烟通过：/health、/v1/models、count_tokens 都正常")
    return 0


if __name__ == "__main__":
    sys.exit(main())
