"""
Pure helpers shared by build-sidecar.sh / build-sidecar.ps1 and build.py.

Everything here is standard library only so the bootstrap Python (before the
build venv exists) can run it. The command line interface at the bottom lets the
shell scripts ask for the production requirements and the pinned PyInstaller
version without re-implementing the parsing in two shell dialects.
"""

import argparse
import hashlib
import re
import sys
import urllib.request
from pathlib import Path
from typing import List, Optional, Sequence, Tuple

PYINSTALLER_VERSION = "6.22.3"
MIN_PYTHON = (3, 10)

CL100K_URL = "https://openaipublic.blob.core.windows.net/encodings/cl100k_base.tiktoken"
CL100K_SHA256 = "223921b76ee99bde995b7ff738513eef100fb51d18c93597a113bcffe865b2a7"

SPEC_KINDS = ("collect-submodules", "hidden-import")
_TESTING_MARKER = re.compile(r"^\s*#\s*testing\b", re.IGNORECASE)

SpecEntry = Tuple[str, str]


class BuildSpecError(Exception):
    """Raised when requirements.txt or hidden-imports.txt cannot be used."""


def prod_requirements(text: str) -> List[str]:
    """
    Extract production requirements from the repository requirements.txt.

    The file lists production packages first and starts its test packages after
    a comment line beginning with "Testing". Everything from that marker on is
    dropped. Comments and blank lines are skipped.

    Args:
        text: Content of requirements.txt.

    Returns:
        Requirement strings in file order.

    Raises:
        BuildSpecError: If no production requirement is found.
    """
    requirements: List[str] = []
    for raw in text.splitlines():
        if _TESTING_MARKER.match(raw):
            break
        line = raw.split("#", 1)[0].strip()
        if line:
            requirements.append(line)
    if not requirements:
        raise BuildSpecError(
            "requirements.txt 里没有找到生产依赖（第一个 'Testing' 注释之前应该有依赖）"
        )
    return requirements


def parse_spec(text: str) -> List[SpecEntry]:
    """
    Parse hidden-imports.txt.

    Args:
        text: File content. Lines are "<kind> <module>"; "#" starts a comment.

    Returns:
        List of (kind, module) tuples in file order.

    Raises:
        BuildSpecError: On an unknown kind or a line without exactly one module.
    """
    entries: List[SpecEntry] = []
    for number, raw in enumerate(text.splitlines(), start=1):
        line = raw.split("#", 1)[0].strip()
        if not line:
            continue
        parts = line.split()
        if parts[0] not in SPEC_KINDS:
            raise BuildSpecError(
                f"hidden-imports.txt 第 {number} 行：不认识的类型 '{parts[0]}'，可用 {', '.join(SPEC_KINDS)}"
            )
        if len(parts) != 2:
            raise BuildSpecError(
                f"hidden-imports.txt 第 {number} 行：需要 '<类型> <模块名>' 两项，实际 {len(parts)} 项"
            )
        entries.append((parts[0], parts[1]))
    return entries


def pyinstaller_args(spec: Sequence[SpecEntry], omit: Optional[str] = None) -> List[str]:
    """
    Turn parsed spec entries into PyInstaller command line arguments.

    Args:
        spec: Entries from parse_spec().
        omit: Module name to leave out (used by the negative build check).

    Returns:
        Flat argument list such as ["--collect-submodules", "uvicorn", ...].

    Raises:
        BuildSpecError: If omit names a module that is not in the spec.
    """
    if omit is not None and omit not in {module for _, module in spec}:
        raise BuildSpecError(f"hidden-imports.txt 里没有 '{omit}'，无法在反向验证里去掉它")
    args: List[str] = []
    for kind, module in spec:
        if module == omit:
            continue
        args += [f"--{kind}", module]
    return args


def cl100k_cache_file_name() -> str:
    """Return the file name tiktoken uses for cl100k_base inside TIKTOKEN_CACHE_DIR (sha1 of the URL)."""
    return hashlib.sha1(CL100K_URL.encode()).hexdigest()


def download_cl100k(target_dir: Path, timeout: float = 60.0) -> Path:
    """
    Download the cl100k_base BPE file into a tiktoken cache directory.

    An existing file with the right sha256 is reused. The download is written to
    a temporary name and renamed only after the hash matches.

    Args:
        target_dir: Directory that becomes TIKTOKEN_CACHE_DIR.
        timeout: Network timeout in seconds.

    Returns:
        Path of the cache file.

    Raises:
        BuildSpecError: If the download fails or the hash does not match.
    """
    target_dir.mkdir(parents=True, exist_ok=True)
    target = target_dir / cl100k_cache_file_name()
    if target.exists() and hashlib.sha256(target.read_bytes()).hexdigest() == CL100K_SHA256:
        return target
    try:
        with urllib.request.urlopen(CL100K_URL, timeout=timeout) as response:
            data = response.read()
    except OSError as exc:
        raise BuildSpecError(f"下载 tiktoken 的 cl100k_base 失败（{exc}），检查网络后重试") from exc
    actual = hashlib.sha256(data).hexdigest()
    if actual != CL100K_SHA256:
        raise BuildSpecError(f"cl100k_base 校验失败（sha256 {actual}），下载内容不完整或被改动")
    partial = target.with_suffix(".part")
    partial.write_bytes(data)
    partial.replace(target)
    return target


def _main(argv: Sequence[str]) -> int:
    """Command line interface used by build-sidecar.sh and build-sidecar.ps1."""
    parser = argparse.ArgumentParser(description="Helpers for the sidecar build scripts")
    sub = parser.add_subparsers(dest="command", required=True)
    req = sub.add_parser("requirements", help="print production requirements, one per line")
    req.add_argument("path", type=Path)
    sub.add_parser("pyinstaller-version", help="print the pinned PyInstaller version")
    sub.add_parser("min-python", help="print the minimum Python version as MAJOR.MINOR")
    pia = sub.add_parser("pyinstaller-args", help="print PyInstaller arguments, one per line")
    pia.add_argument("spec", type=Path)
    pia.add_argument("--omit", default=None, help="module to leave out (negative check)")
    dl = sub.add_parser("download-tiktoken", help="download cl100k_base into a cache directory")
    dl.add_argument("cache_dir", type=Path)
    args = parser.parse_args(argv)
    try:
        if args.command == "requirements":
            print("\n".join(prod_requirements(args.path.read_text(encoding="utf-8"))))
        elif args.command == "pyinstaller-version":
            print(PYINSTALLER_VERSION)
        elif args.command == "min-python":
            print(f"{MIN_PYTHON[0]}.{MIN_PYTHON[1]}")
        elif args.command == "pyinstaller-args":
            spec = parse_spec(args.spec.read_text(encoding="utf-8"))
            print("\n".join(pyinstaller_args(spec, omit=args.omit or None)))
        else:
            path = download_cl100k(args.cache_dir)
            print(f"{path.name} ({path.stat().st_size} bytes)")
    except (BuildSpecError, OSError) as exc:
        print(f"错误：{exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(_main(sys.argv[1:]))
