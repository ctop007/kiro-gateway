#!/usr/bin/env python3
"""Helpers behind desktop/scripts/release.sh (tasks 8.7).

Pure functions (version bump, manifest, upload plan, ordered upload) plus a
small CLI that release.sh calls. Standard library only, except the real COS
uploader which imports ``qcloud_cos`` lazily.

Run with ``/opt/miniconda3/bin/python -I release_lib.py <command> ...``.
Credentials are read from the environment or ``~/.tccli/default.credential``
and are never printed.
"""

import argparse
import datetime as dt
import json
import os
import re
import shutil
import sys
import urllib.error
import urllib.request
from dataclasses import dataclass
from pathlib import Path
from typing import Callable, Dict, List, Optional, Tuple

BUCKET = "tools-1254376798"
REGION = "ap-guangzhou"
PREFIX = "kiro-gateway/desktop"
BASE_URL = f"https://{BUCKET}.cos.{REGION}.myqcloud.com"
MANIFEST_KEY = f"{PREFIX}/latest.json"
DEFAULT_MIN_REQUIRED = "0.0.1"

VERSION_RE = re.compile(r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$")


class ReleaseError(Exception):
    """A problem the maintainer can fix; the message is shown as is."""


def parse_version(text: str) -> Optional[Tuple[int, int, int]]:
    """Parses a plain x.y.z version; None for anything else."""
    m = VERSION_RE.match(text or "")
    return tuple(int(g) for g in m.groups()) if m else None


def require_version(text: str, what: str = "VERSION") -> Tuple[int, int, int]:
    parsed = parse_version(text)
    if parsed is None:
        raise ReleaseError(f"{what} 必须是 x.y.z 形式（例如 0.0.2），收到：{text!r}")
    return parsed


# ---------------------------------------------------------------- versions

_PKG_RE = re.compile(r'(?m)^(  "version": ")([^"]*)(")')
_CARGO_VERSION_RE = re.compile(r'(?m)^(version\s*=\s*")([^"]*)(")')
_LOCK_RE = re.compile(r'(\[\[package\]\]\nname = "kiro-gateway-desktop"\nversion = ")([^"]*)(")')


def _package_section(text: str) -> Tuple[int, int]:
    start = text.find("[package]")
    if start < 0:
        raise ReleaseError("Cargo.toml 里没有 [package]")
    nxt = re.search(r"(?m)^\[", text[start + len("[package]"):])
    end = len(text) if nxt is None else start + len("[package]") + nxt.start()
    return start, end


def read_versions(root: Path) -> Dict[str, str]:
    """Current version in each of the three files under the desktop/ root."""
    out: Dict[str, str] = {}
    pkg = _PKG_RE.search((root / "package.json").read_text(encoding="utf-8"))
    conf = _PKG_RE.search((root / "src-tauri/tauri.conf.json").read_text(encoding="utf-8"))
    cargo = (root / "src-tauri/Cargo.toml").read_text(encoding="utf-8")
    s, e = _package_section(cargo)
    cv = _CARGO_VERSION_RE.search(cargo[s:e])
    for name, m in (("package.json", pkg), ("tauri.conf.json", conf), ("Cargo.toml", cv)):
        if m is None:
            raise ReleaseError(f"{name} 里找不到 version 字段")
        out[name] = m.group(2)
    return out


def bump_versions(root: Path, version: str, write: bool = True) -> List[str]:
    """Sets the version in package.json, tauri.conf.json, Cargo.toml (and the
    package entry of Cargo.lock when present).

    Raises:
        ReleaseError: invalid version, not newer than the current one, or the
            three files disagree. Nothing is written in that case; all new
            contents are computed before the first write.

    Returns:
        Human readable change lines (also returned when ``write`` is False).
    """
    new = require_version(version)
    current = read_versions(root)
    if len(set(current.values())) != 1:
        raise ReleaseError(f"三处版本号不一致：{current}")
    cur_text = next(iter(current.values()))
    cur = require_version(cur_text, "当前版本")
    if new <= cur:
        raise ReleaseError(f"新版本 {version} 必须高于当前版本 {cur_text}")

    edits: List[Tuple[Path, str]] = []
    changes: List[str] = []

    def sub(path: Path, pattern: "re.Pattern[str]", text: str, label: str) -> str:
        result, n = pattern.subn(lambda m: m.group(1) + version + m.group(3), text, count=1)
        if n != 1:
            raise ReleaseError(f"{label} 里找不到可替换的 version")
        changes.append(f"{label}: {cur_text} -> {version}")
        return result

    p = root / "package.json"
    edits.append((p, sub(p, _PKG_RE, p.read_text(encoding="utf-8"), "package.json")))
    p = root / "src-tauri/tauri.conf.json"
    edits.append((p, sub(p, _PKG_RE, p.read_text(encoding="utf-8"), "tauri.conf.json")))
    p = root / "src-tauri/Cargo.toml"
    cargo = p.read_text(encoding="utf-8")
    s, e = _package_section(cargo)
    section = sub(p, _CARGO_VERSION_RE, cargo[s:e], "Cargo.toml")
    edits.append((p, cargo[:s] + section + cargo[e:]))
    lock = root / "src-tauri/Cargo.lock"
    if lock.is_file():
        text = lock.read_text(encoding="utf-8")
        if _LOCK_RE.search(text):
            edits.append((lock, sub(lock, _LOCK_RE, text, "Cargo.lock")))
    for path, text in edits:
        if path.name in ("package.json", "tauri.conf.json"):
            json.loads(text)  # still valid JSON
    if write:
        for path, text in edits:
            tmp = path.with_name(path.name + ".release-tmp")
            tmp.write_text(text, encoding="utf-8")
            os.replace(tmp, path)
    return changes


VERSION_FILES = ("package.json", "src-tauri/tauri.conf.json", "src-tauri/Cargo.toml", "src-tauri/Cargo.lock")


def snapshot_version_files(root: Path, dest: Path) -> List[str]:
    """Copies the files ``bump_versions`` may touch into ``dest`` (byte for byte).

    Returns:
        Relative names that were saved (Cargo.lock is skipped when absent).
    """
    dest.mkdir(parents=True, exist_ok=True)
    saved: List[str] = []
    for rel in VERSION_FILES:
        src = root / rel
        if src.is_file():
            target = dest / rel
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(src, target)
            saved.append(rel)
    return saved


def restore_version_files(root: Path, snapshot: Path) -> List[str]:
    """Puts back what ``snapshot_version_files`` saved and removes leftover temp files.

    Returns:
        Relative names that were restored.
    """
    restored: List[str] = []
    for rel in VERSION_FILES:
        saved = snapshot / rel
        target = root / rel
        leftover = target.with_name(target.name + ".release-tmp")
        if leftover.exists():
            leftover.unlink()
        if saved.is_file():
            tmp = target.with_name(target.name + ".release-restore")
            shutil.copyfile(saved, tmp)
            os.replace(tmp, target)
            restored.append(rel)
    return restored


# ---------------------------------------------------------------- artifacts


def _one(paths: List[Path], what: str, where: Path) -> Path:
    if len(paths) != 1:
        raise ReleaseError(f"在 {where} 下需要恰好一个 {what}，实际 {len(paths)} 个")
    return paths[0]


def collect_artifacts(version: str, mac_bundle: Optional[Path], win_bundle: Optional[Path]) -> Dict[str, Path]:
    """Finds the build outputs for ``version``.

    Args:
        version: Release version; file names must carry it (stale outputs of an
            older build are rejected).
        mac_bundle: ``.../universal-apple-darwin/release/bundle`` or None.
        win_bundle: A ``bundle`` directory with ``nsis/`` from a Windows build, or None.

    Returns:
        Map with keys ``mac_package``, ``mac_package_sig``, ``mac_dmg`` and,
        when ``win_bundle`` is given, ``win_setup``, ``win_setup_sig``.
    """
    out: Dict[str, Path] = {}
    if mac_bundle is None and win_bundle is None:
        raise ReleaseError("没有可发布的产物目录")
    if mac_bundle is not None:
        macos = mac_bundle / "macos"
        pkg = _one(sorted(macos.glob("*.app.tar.gz")), "*.app.tar.gz", macos)
        sig = pkg.with_name(pkg.name + ".sig")
        dmgs = sorted((mac_bundle / "dmg").glob(f"*_{version}_*.dmg"))
        dmg = _one(dmgs, f"版本 {version} 的 dmg", mac_bundle / "dmg")
        if not sig.is_file():
            raise ReleaseError(f"缺少签名文件 {sig}（构建时没设置 TAURI_SIGNING_PRIVATE_KEY？）")
        out.update(mac_package=pkg, mac_package_sig=sig, mac_dmg=dmg)
    if win_bundle is not None:
        nsis = win_bundle / "nsis"
        setup = _one(sorted(nsis.glob(f"*_{version}_x64-setup.exe")), f"版本 {version} 的 setup.exe", nsis)
        sig = setup.with_name(setup.name + ".sig")
        if not sig.is_file():
            raise ReleaseError(f"缺少签名文件 {sig}")
        out.update(win_setup=setup, win_setup_sig=sig)
    return out


@dataclass(frozen=True)
class Step:
    """One upload."""

    kind: str  # installer | package | sig | manifest
    local: str
    key: str
    content_type: str
    cache_control: Optional[str] = None
    size: int = 0

    @property
    def url(self) -> str:
        return f"{BASE_URL}/{self.key}"


def canonical_names(version: str) -> Dict[str, Tuple[str, str, str]]:
    """(canonical file name, kind, content type) per artifact key."""
    setup = f"Kiro-Gateway_{version}_x64-setup.exe"
    dmg = f"Kiro-Gateway_{version}_universal.dmg"
    return {
        "mac_dmg": (dmg, "installer", "application/x-apple-diskimage"),
        "win_setup": (setup, "installer", "application/vnd.microsoft.portable-executable"),
        "mac_package": ("Kiro-Gateway.app.tar.gz", "package", "application/gzip"),
        "mac_package_sig": ("Kiro-Gateway.app.tar.gz.sig", "sig", "text/plain; charset=utf-8"),
        "win_setup_sig": (setup + ".sig", "sig", "text/plain; charset=utf-8"),
    }


def build_plan(version: str, staged: Dict[str, Path], manifest_path: Path) -> List[Step]:
    """Upload order: installers, update packages, .sig files, latest.json last.

    The manifest is the only object clients poll, so it goes up only after
    every file it points to is in place (and checked with HEAD).
    """
    names = canonical_names(version)
    order = ["mac_dmg", "win_setup", "mac_package", "mac_package_sig", "win_setup_sig"]
    steps: List[Step] = []
    for key in order:
        if key not in staged:
            continue
        name, kind, ctype = names[key]
        path = staged[key]
        steps.append(Step(kind, str(path), f"{PREFIX}/{version}/{name}", ctype, None, path.stat().st_size))
    steps.append(
        Step("manifest", str(manifest_path), MANIFEST_KEY, "application/json; charset=utf-8",
             "no-cache, max-age=0", manifest_path.stat().st_size)
    )
    return steps


def stage_artifacts(version: str, found: Dict[str, Path], out_dir: Path) -> Dict[str, Path]:
    """Copies artifacts to ``out_dir`` under their canonical names."""
    names = canonical_names(version)
    out_dir.mkdir(parents=True, exist_ok=True)
    staged: Dict[str, Path] = {}
    for key, src in found.items():
        dest = out_dir / names[key][0]
        shutil.copyfile(src, dest)
        staged[key] = dest
    return staged


# ---------------------------------------------------------------- manifest


def read_signature(path: Path) -> str:
    text = path.read_text(encoding="utf-8").strip()
    if not text:
        raise ReleaseError(f"签名文件是空的：{path.name}")
    return text


def build_manifest(
    version: str,
    notes: str,
    pub_date: str,
    min_required: str,
    staged: Dict[str, Path],
) -> dict:
    """Builds latest.json (design decision 11 plus ``manualDownload``)."""
    require_version(version)
    require_version(min_required, "minRequiredVersion")
    if parse_version(min_required) > parse_version(version):
        raise ReleaseError(f"minRequiredVersion {min_required} 不能高于发布版本 {version}")
    names = canonical_names(version)

    def url(key: str) -> str:
        return f"{BASE_URL}/{PREFIX}/{version}/{names[key][0]}"

    platforms: Dict[str, dict] = {}
    manual: Dict[str, str] = {}
    if "mac_package" in staged:
        entry = {"url": url("mac_package"), "signature": read_signature(staged["mac_package_sig"])}
        platforms["darwin-aarch64"] = dict(entry)
        platforms["darwin-x86_64"] = dict(entry)
        manual["darwin"] = url("mac_dmg")
    if "win_setup" in staged:
        platforms["windows-x86_64"] = {"url": url("win_setup"), "signature": read_signature(staged["win_setup_sig"])}
        manual["windows"] = url("win_setup")
    if not platforms:
        raise ReleaseError("清单里没有任何平台")
    return {
        "version": version,
        "minRequiredVersion": min_required,
        "notes": notes,
        "pub_date": pub_date,
        "platforms": platforms,
        "manualDownload": manual,
    }


def validate_manifest(m: object) -> List[str]:
    """Returns a list of problems; empty means the manifest is complete."""
    problems: List[str] = []
    if not isinstance(m, dict):
        return ["清单不是 JSON 对象"]
    version = m.get("version")
    if not isinstance(version, str) or parse_version(version) is None:
        problems.append("version 缺失或不是 x.y.z")
    mn = m.get("minRequiredVersion")
    if not isinstance(mn, str) or parse_version(mn) is None:
        problems.append("minRequiredVersion 缺失或不是 x.y.z")
    elif isinstance(version, str) and parse_version(version) and parse_version(mn) > parse_version(version):
        problems.append("minRequiredVersion 高于 version")
    if not isinstance(m.get("notes"), str):
        problems.append("notes 缺失")
    pub = m.get("pub_date")
    try:
        dt.datetime.fromisoformat(pub)  # type: ignore[arg-type]
    except (TypeError, ValueError):
        problems.append("pub_date 缺失或不是 ISO 8601")
    platforms = m.get("platforms")
    if not isinstance(platforms, dict) or not platforms:
        problems.append("platforms 缺失")
    else:
        for key, entry in platforms.items():
            if key not in ("darwin-aarch64", "darwin-x86_64", "windows-x86_64"):
                problems.append(f"未知平台 {key}")
            if not (isinstance(entry, dict) and str(entry.get("url", "")).startswith("https://")
                    and str(entry.get("signature", "")).strip()):
                problems.append(f"platforms.{key} 缺 url 或 signature")
    manual = m.get("manualDownload")
    if not isinstance(manual, dict) or not manual:
        problems.append("manualDownload 缺失")
    elif isinstance(platforms, dict):
        if any(k.startswith("darwin") for k in platforms) and not str(manual.get("darwin", "")).startswith("https://"):
            problems.append("manualDownload.darwin 缺失")
        if "windows-x86_64" in platforms and not str(manual.get("windows", "")).startswith("https://"):
            problems.append("manualDownload.windows 缺失")
    return problems


def resolve_min_required(version: str, force: bool, previous_min: Optional[str]) -> str:
    """FORCE sets the floor to this release; otherwise keep the previous floor
    (so an earlier forced upgrade is not silently lifted), never above ``version``."""
    if force:
        return version
    floor = previous_min if previous_min and parse_version(previous_min) else DEFAULT_MIN_REQUIRED
    return floor if parse_version(floor) <= parse_version(version) else version


def published_manifest_url() -> str:
    """Address of the live manifest; ``RELEASE_MANIFEST_URL`` overrides it (tests, read only)."""
    return os.environ.get("RELEASE_MANIFEST_URL") or f"{BASE_URL}/{MANIFEST_KEY}"


NO_PUBLISHED_MANIFEST_MESSAGE = (
    "无法读取线上 latest.json，为避免降低最低可用版本已中止；确认是首次发布请加 FIRST_RELEASE=1"
)


def previous_min_for_release(
    url: str,
    force: bool,
    first_release: bool,
    fetch: Optional[Callable[[str], Optional[str]]] = None,
) -> Optional[str]:
    """minRequiredVersion of the published manifest, which a non-forced release must carry over.

    Args:
        url: Address of the published ``latest.json``.
        force: FORCE release: the floor becomes the new version, nothing to carry over.
        first_release: Nothing is published yet, so there is no floor to keep.
        fetch: Reader of the published floor (defaults to ``fetch_previous_min``).

    Returns:
        The published floor, or None when ``force`` or ``first_release`` skip the lookup.

    Raises:
        ReleaseError: the published manifest cannot be read (offline, 404, no valid
            ``minRequiredVersion``). Falling back to the default floor would silently
            lift an earlier forced upgrade.
    """
    if force or first_release:
        return None
    previous = (fetch or fetch_previous_min)(url)
    if previous is None:
        raise ReleaseError(NO_PUBLISHED_MANIFEST_MESSAGE)
    return previous


def confirm_force(version: str, read: Callable[[], str]) -> bool:
    """FORCE=1 needs the version typed back exactly."""
    return read().strip() == version


# ---------------------------------------------------------------- upload


def load_credentials(env: Dict[str, str], home: Path) -> Tuple[str, str]:
    """COS_SECRET_ID / COS_SECRET_KEY, else ~/.tccli/default.credential."""
    sid, skey = env.get("COS_SECRET_ID", ""), env.get("COS_SECRET_KEY", "")
    if sid and skey:
        return sid, skey
    path = home / ".tccli" / "default.credential"
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
        sid, skey = data.get("secretId", ""), data.get("secretKey", "")
    except (OSError, ValueError, AttributeError):
        sid = skey = ""
    if not (sid and skey):
        raise ReleaseError("找不到 COS 凭证：设置 COS_SECRET_ID / COS_SECRET_KEY，或配置 ~/.tccli/default.credential")
    return sid, skey


class CosUploader:
    """Uploads with qcloud_cos. Credentials stay inside this object."""

    def __init__(self, secret_id: str, secret_key: str) -> None:
        try:
            from qcloud_cos import CosConfig, CosS3Client
        except ImportError as exc:
            raise ReleaseError("缺少 qcloud_cos，请用装了它的 Python（/opt/miniconda3/bin/python）运行") from exc
        config = CosConfig(Region=REGION, SecretId=secret_id, SecretKey=secret_key, Scheme="https")
        self._client = CosS3Client(config)

    def put(self, step: Step) -> None:
        kwargs = {"ContentType": step.content_type}
        if step.cache_control:
            kwargs["CacheControl"] = step.cache_control
        self._client.upload_file(Bucket=BUCKET, Key=step.key, LocalFilePath=step.local, **kwargs)


def head_url(url: str, timeout: float = 20.0) -> Tuple[int, Optional[int]]:
    """(status, content-length) of a HEAD request; status 0 on network failure."""
    req = urllib.request.Request(url, method="HEAD")
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            length = resp.headers.get("Content-Length")
            return resp.status, int(length) if length and length.isdigit() else None
    except urllib.error.HTTPError as exc:
        return exc.code, None
    except (urllib.error.URLError, OSError):
        return 0, None


def run_upload(
    plan: List[Step],
    put: Callable[[Step], None],
    head: Callable[[str], Tuple[int, Optional[int]]],
    log: Callable[[str], None] = print,
) -> None:
    """Executes ``plan`` in order; the manifest only after every other file passed HEAD.

    Raises:
        ReleaseError: a HEAD check failed; the manifest has NOT been uploaded.
    """
    if not plan or plan[-1].kind != "manifest" or any(s.kind == "manifest" for s in plan[:-1]):
        raise ReleaseError("上传计划必须以 latest.json 结尾，且只有一个")
    files, manifest = plan[:-1], plan[-1]
    for step in files:
        log(f"上传 {step.key}")
        put(step)
    for step in files:
        status, length = head(step.url)
        if status != 200:
            raise ReleaseError(f"HEAD 校验失败（HTTP {status}）：{step.key}。latest.json 没有更新")
        if length is not None and length != step.size:
            raise ReleaseError(f"HEAD 校验失败（大小 {length} 与本地 {step.size} 不符）：{step.key}。latest.json 没有更新")
        log(f"已确认 {step.key}")
    log(f"上传 {manifest.key}")
    put(manifest)


# ---------------------------------------------------------------- CLI


def fetch_previous_min(url: str, timeout: float = 15.0) -> Optional[str]:
    """minRequiredVersion of the currently published manifest, None if unavailable."""
    try:
        with urllib.request.urlopen(url, timeout=timeout) as resp:
            data = json.loads(resp.read().decode("utf-8"))
        value = data.get("minRequiredVersion")
        return value if isinstance(value, str) and parse_version(value) else None
    except (urllib.error.URLError, OSError, ValueError, AttributeError):
        return None


def _notes_from(arg: str) -> str:
    if not arg:
        raise ReleaseError("NOTES 不能为空：传更新说明文件路径或一行文字")
    p = Path(arg)
    text = p.read_text(encoding="utf-8") if p.is_file() else arg
    text = text.strip()
    if not text:
        raise ReleaseError("NOTES 不能为空")
    return text


def make_fake_artifacts(version: str, bundle: Path) -> None:
    """Placeholder outputs for --dry-run without a real build."""
    (bundle / "macos").mkdir(parents=True, exist_ok=True)
    (bundle / "dmg").mkdir(parents=True, exist_ok=True)
    (bundle / "macos/Kiro Gateway.app.tar.gz").write_bytes(b"dry-run placeholder package")
    (bundle / "macos/Kiro Gateway.app.tar.gz.sig").write_text("DRY-RUN-PLACEHOLDER-SIGNATURE\n", encoding="utf-8")
    (bundle / "dmg" / f"Kiro Gateway_{version}_universal.dmg").write_bytes(b"dry-run placeholder dmg")


def cmd_prepare(args: argparse.Namespace) -> int:
    version = args.version
    require_version(version)
    notes = _notes_from(args.notes)
    mac = Path(args.bundle_dir) if args.bundle_dir else None
    win = Path(args.windows_dir) if args.windows_dir else None
    found = collect_artifacts(version, mac, win)
    out = Path(args.out)
    staged = stage_artifacts(version, found, out)
    prev_min = None
    if not args.no_fetch:
        prev_min = previous_min_for_release(published_manifest_url(), args.force, args.first_release)
    min_required = resolve_min_required(version, args.force, prev_min)
    pub_date = dt.datetime.now().astimezone().replace(microsecond=0).isoformat()
    manifest = build_manifest(version, notes, pub_date, min_required, staged)
    problems = validate_manifest(manifest)
    if problems:
        raise ReleaseError("生成的清单不完整：" + "；".join(problems))
    manifest_path = out / "latest.json"
    manifest_path.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    plan = build_plan(version, staged, manifest_path)
    (out / "plan.json").write_text(json.dumps([s.__dict__ for s in plan], ensure_ascii=False, indent=2), encoding="utf-8")
    print(f"版本 {version}，minRequiredVersion {min_required}{'（FORCE）' if args.force else ''}")
    print(f"清单：{manifest_path}")
    print("上传顺序：")
    for i, s in enumerate(plan, 1):
        print(f"  {i}. [{s.kind}] {s.key}  ({s.size} 字节)")
    print("  HEAD 校验位于清单之前")
    return 0


def cmd_upload(args: argparse.Namespace) -> int:
    out = Path(args.staging)
    plan = [Step(**d) for d in json.loads((out / "plan.json").read_text(encoding="utf-8"))]
    problems = validate_manifest(json.loads(Path(plan[-1].local).read_text(encoding="utf-8")))
    if problems:
        raise ReleaseError("清单不完整，拒绝上传：" + "；".join(problems))
    sid, skey = load_credentials(dict(os.environ), Path.home())
    uploader = CosUploader(sid, skey)
    run_upload(plan, uploader.put, head_url)
    print("完成。")
    return 0


def main(argv: Optional[List[str]] = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    sub = ap.add_subparsers(dest="cmd", required=True)

    p = sub.add_parser("current-version")
    p.add_argument("--root", required=True)
    p = sub.add_parser("bump")
    p.add_argument("--root", required=True)
    p.add_argument("--version", required=True)
    p.add_argument("--dry-run", action="store_true")
    p = sub.add_parser("check-version")
    p.add_argument("--root", required=True)
    p.add_argument("--version", required=True)
    p = sub.add_parser("fake-artifacts")
    p.add_argument("--version", required=True)
    p.add_argument("--bundle-dir", required=True)
    p = sub.add_parser("prepare")
    p.add_argument("--version", required=True)
    p.add_argument("--notes", required=True)
    p.add_argument("--force", action="store_true")
    p.add_argument("--bundle-dir")
    p.add_argument("--windows-dir")
    p.add_argument("--out", required=True)
    p.add_argument("--no-fetch", action="store_true", help="do not read the published manifest")
    p.add_argument("--first-release", action="store_true", help="nothing is published yet: no floor to keep")
    p = sub.add_parser("check-published")
    p.add_argument("--force", action="store_true")
    p.add_argument("--first-release", action="store_true")
    p = sub.add_parser("snapshot")
    p.add_argument("--root", required=True)
    p.add_argument("--dir", required=True)
    p = sub.add_parser("restore")
    p.add_argument("--root", required=True)
    p.add_argument("--dir", required=True)
    p = sub.add_parser("upload")
    p.add_argument("--staging", required=True)
    p = sub.add_parser("confirm-force")
    p.add_argument("--version", required=True)
    p = sub.add_parser("verify-manifest")
    p.add_argument("path")

    args = ap.parse_args(argv)
    try:
        if args.cmd == "current-version":
            versions = read_versions(Path(args.root))
            if len(set(versions.values())) != 1:
                raise ReleaseError(f"三处版本号不一致：{versions}")
            print(next(iter(versions.values())))
            return 0
        if args.cmd == "check-version":
            # Same checks as bump, nothing written.
            for line in bump_versions(Path(args.root), args.version, write=False):
                print(f"将修改 {line}")
            return 0
        if args.cmd == "bump":
            for line in bump_versions(Path(args.root), args.version, write=not args.dry_run):
                print(line)
            return 0
        if args.cmd == "check-published":
            previous_min_for_release(published_manifest_url(), args.force, args.first_release)
            return 0
        if args.cmd == "snapshot":
            snapshot_version_files(Path(args.root), Path(args.dir))
            return 0
        if args.cmd == "restore":
            for rel in restore_version_files(Path(args.root), Path(args.dir)):
                print(f"已恢复 {rel}")
            return 0
        if args.cmd == "fake-artifacts":
            make_fake_artifacts(args.version, Path(args.bundle_dir))
            return 0
        if args.cmd == "prepare":
            return cmd_prepare(args)
        if args.cmd == "upload":
            return cmd_upload(args)
        if args.cmd == "confirm-force":
            sys.stdout.write(f"强制升级会让所有低于 {args.version} 的旧版立刻无法使用。\n再输入一遍版本号 {args.version} 确认：")
            sys.stdout.flush()
            return 0 if confirm_force(args.version, sys.stdin.readline) else 1
        if args.cmd == "verify-manifest":
            problems = validate_manifest(json.loads(Path(args.path).read_text(encoding="utf-8")))
            for p_ in problems:
                print(p_, file=sys.stderr)
            return 1 if problems else 0
    except ReleaseError as exc:
        print(f"错误：{exc}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
