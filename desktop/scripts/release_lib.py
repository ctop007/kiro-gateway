#!/usr/bin/env python3
"""Helpers for the desktop release (design decision 11, GitHub Releases edition).

Used by the ``release`` job of .github/workflows/desktop-release.yml: stage the
build outputs under space-free names, build ``latest.json`` whose URLs point at
the assets of that very GitHub Release, and write the release notes. Also offers
``bump`` / ``check-version`` for the three version files. Standard library only.

Run with ``python3 -I release_lib.py <command> ...``.
"""

import argparse
import datetime as dt
import json
import os
import re
import shutil
import sys
from pathlib import Path
from typing import Dict, List, Optional, Tuple

DEFAULT_MIN_REQUIRED = "0.0.1"
FORCE_MARKER = "[force-upgrade]"

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


def canonical_names(version: str) -> Dict[str, str]:
    """Space-free file name per artifact key.

    These are the names of the files uploaded to the GitHub Release and the ones
    ``latest.json`` points at (GitHub rewrites spaces in asset names, so the raw
    build names with spaces must never be uploaded).
    """
    setup = f"Kiro-Gateway_{version}_x64-setup.exe"
    return {
        "mac_dmg": f"Kiro-Gateway_{version}_universal.dmg",
        "win_setup": setup,
        "mac_package": "Kiro-Gateway.app.tar.gz",
        "mac_package_sig": "Kiro-Gateway.app.tar.gz.sig",
        "win_setup_sig": setup + ".sig",
    }


def stage_artifacts(version: str, found: Dict[str, Path], out_dir: Path) -> Dict[str, Path]:
    """Copies artifacts to ``out_dir`` under their canonical names."""
    names = canonical_names(version)
    out_dir.mkdir(parents=True, exist_ok=True)
    staged: Dict[str, Path] = {}
    for key, src in found.items():
        dest = out_dir / names[key]
        shutil.copyfile(src, dest)
        staged[key] = dest
    return staged


# ---------------------------------------------------------------- manifest


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
    base_url: str,
) -> dict:
    """Builds latest.json (design decision 11 plus ``manualDownload``).

    Args:
        version: Release version (x.y.z).
        notes: Update notes shown to users.
        pub_date: ISO 8601 publication time.
        min_required: Lowest version that may keep running.
        staged: Output of ``stage_artifacts``.
        base_url: Directory URL of the release assets, e.g.
            ``https://github.com/<owner>/<repo>/releases/download/<tag>``; every
            URL in the manifest is ``<base_url>/<staged file name>``.

    Raises:
        ReleaseError: bad version, floor above the version, empty signature, no platform,
            or a ``base_url`` that is not https.
    """
    require_version(version)
    require_version(min_required, "minRequiredVersion")
    if parse_version(min_required) > parse_version(version):
        raise ReleaseError(f"minRequiredVersion {min_required} 不能高于发布版本 {version}")
    base = (base_url or "").strip().rstrip("/")
    if not base.startswith("https://"):
        raise ReleaseError(f"--base-url 必须以 https:// 开头，收到：{base_url!r}")
    names = canonical_names(version)

    def url(key: str) -> str:
        return f"{base}/{names[key]}"

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


def split_tag_notes(raw: str) -> Tuple[str, bool]:
    """Splits an annotated tag body into update notes and the force flag.

    A line that is exactly ``[force-upgrade]`` turns the force flag on and is
    removed from the notes (users never see the marker).

    Raises:
        ReleaseError: nothing is left to show as notes.
    """
    lines = (raw or "").replace("\r\n", "\n").split("\n")
    force = any(line == FORCE_MARKER for line in lines)
    notes = "\n".join(line for line in lines if line != FORCE_MARKER).strip()
    if not notes:
        raise ReleaseError("更新说明是空的：用 git tag -a <tag> -F notes.md 打 tag，正文写更新说明")
    return notes, force


def read_previous_min(path: Optional[Path]) -> Optional[str]:
    """``minRequiredVersion`` of the previously published ``latest.json``.

    Args:
        path: Local copy of the previous manifest, or None when it could not be fetched.

    Returns:
        The floor, or None when the file is missing, unreadable or has no valid floor.
    """
    if path is None or not path.is_file():
        return None
    try:
        value = json.loads(path.read_text(encoding="utf-8")).get("minRequiredVersion")
    except (OSError, ValueError, AttributeError):
        return None
    return value if isinstance(value, str) and parse_version(value) else None


def check_manifest_assets(manifest: dict, assets_dir: Path, base_url: str) -> List[str]:
    """Problems that would make the manifest point at files the Release will not have.

    Every URL in the manifest must be ``<base_url>/<name>`` with ``<name>`` an existing
    file in ``assets_dir`` (the directory that is uploaded to the Release).
    """
    base = base_url.strip().rstrip("/") + "/"
    urls = [e.get("url", "") for e in manifest.get("platforms", {}).values()]
    urls += list(manifest.get("manualDownload", {}).values())
    problems: List[str] = []
    for url in urls:
        if not str(url).startswith(base):
            problems.append(f"{url} 不在 {base} 下")
            continue
        name = str(url)[len(base):]
        if "/" in name or " " in name or not (assets_dir / name).is_file():
            problems.append(f"Release 里不会有 {name}")
    return problems


# ---------------------------------------------------------------- CLI


def _read_notes(args: argparse.Namespace) -> Tuple[str, bool]:
    if args.tag_body:
        return split_tag_notes(Path(args.tag_body).read_text(encoding="utf-8"))
    if not args.notes:
        raise ReleaseError("需要 --tag-body（tag 正文文件）或 --notes（文件路径或一行文字）")
    p = Path(args.notes)
    text = (p.read_text(encoding="utf-8") if p.is_file() else args.notes).strip()
    if not text:
        raise ReleaseError("NOTES 不能为空")
    return text, False


def cmd_prepare(args: argparse.Namespace) -> int:
    """Stages assets, writes ``<out>/assets/`` (upload this directory) and ``<out>/notes.md``."""
    version = args.version
    require_version(version)
    notes, tag_force = _read_notes(args)
    force = args.force or tag_force
    mac = Path(args.bundle_dir) if args.bundle_dir else None
    win = Path(args.windows_dir) if args.windows_dir else None
    found = collect_artifacts(version, mac, win)
    out = Path(args.out)
    assets = out / "assets"
    staged = stage_artifacts(version, found, assets)
    previous = read_previous_min(Path(args.previous_manifest)) if args.previous_manifest else None
    if force:
        print(f"强制升级：minRequiredVersion = {version}")
    elif previous is None:
        print(f"没有取到上一个已发布版本的 latest.json，minRequiredVersion 用默认值 {DEFAULT_MIN_REQUIRED}")
    else:
        print(f"沿用上一个已发布版本的 minRequiredVersion {previous}")
    min_required = resolve_min_required(version, force, previous)
    pub_date = dt.datetime.now().astimezone().replace(microsecond=0).isoformat()
    manifest = build_manifest(version, notes, pub_date, min_required, staged, args.base_url)
    problems = validate_manifest(manifest) + check_manifest_assets(manifest, assets, args.base_url)
    if problems:
        raise ReleaseError("生成的清单不完整：" + "；".join(problems))
    (assets / "latest.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    (out / "notes.md").write_text(notes + "\n", encoding="utf-8")
    print(f"版本 {version}，minRequiredVersion {min_required}")
    print("将上传到 Release 的文件：")
    for p in sorted(assets.iterdir()):
        print(f"  {p.name}  ({p.stat().st_size} 字节)")
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
    p = sub.add_parser("prepare", help="stage assets and write latest.json for a GitHub Release")
    p.add_argument("--version", required=True)
    p.add_argument("--base-url", required=True, help="https URL of the Release's asset directory")
    p.add_argument("--tag-body", help="file with the annotated tag body (a [force-upgrade] line forces)")
    p.add_argument("--notes", help="update notes: a file path or one line of text")
    p.add_argument("--force", action="store_true")
    p.add_argument("--bundle-dir")
    p.add_argument("--windows-dir")
    p.add_argument("--previous-manifest", help="local copy of the previously published latest.json")
    p.add_argument("--out", required=True)
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
            for line in bump_versions(Path(args.root), args.version, write=False):
                print(f"将修改 {line}")
            return 0
        if args.cmd == "bump":
            for line in bump_versions(Path(args.root), args.version, write=not args.dry_run):
                print(line)
            return 0
        if args.cmd == "prepare":
            return cmd_prepare(args)
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
