#!/usr/bin/env bash
# Build the kiro-gateway sidecar (PyInstaller --onedir) into
# desktop/src-tauri/resources/gateway-<arch>/ and smoke-test it.
#
# Usage:
#   scripts/build-sidecar.sh                      build for this machine's architecture, smoke, publish
#   scripts/build-sidecar.sh --arch arm64|x86_64  build for that architecture (aarch64 is accepted as arm64).
#                                                 x86_64 on an Apple Silicon Mac re-runs itself under
#                                                 `arch -x86_64` (Rosetta) with an x86_64 interpreter.
#   scripts/build-sidecar.sh --negative-check M   build WITHOUT hidden-import M into a temp dir
#                                                 and succeed only if the smoke check fails
#                                                 (proves the smoke check can catch a missing import)
#
# Layout per architecture (a universal macOS App ships both):
#   arm64   venv .build-venv           output resources/gateway-aarch64/
#   x86_64  venv .build-venv-x86_64    output resources/gateway-x86_64/
#
# Environment:
#   KGW_PYTHON   interpreter used to create the build venv (default: first of
#                python3.13 .. python3.10, python3 that is >= 3.10; for a non-native
#                --arch, `uv python find cpython-3.13-macos-<arch>` is tried first).
#                Its architecture must match --arch.
#   KGW_SMOKE_TIMEOUT   seconds the smoke check waits for /health (default 60)
#
# The output directory is only replaced after the smoke check passes, so a
# failed build never leaves a half-working sidecar behind.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DESKTOP_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_ROOT="$(cd "$DESKTOP_DIR/.." && pwd)"
SIDECAR_SCRIPTS="$SCRIPT_DIR/sidecar"
APP_NAME="kiro-gateway"
SMOKE_TIMEOUT="${KGW_SMOKE_TIMEOUT:-60}"

# Normalise an architecture name to the Rust/Tauri spelling (aarch64 | x86_64).
normalize_arch() {
  case "$1" in
    arm64|aarch64) echo aarch64 ;;
    x86_64|amd64|x64) echo x86_64 ;;
    *) return 1 ;;
  esac
}

NATIVE_ARCH="$(normalize_arch "$(uname -m)" || true)"
TARGET_ARCH="$NATIVE_ARCH"
NEGATIVE_MODULE=""
ORIG_ARGS=("$@")
while [ $# -gt 0 ]; do
  case "$1" in
    --negative-check)
      NEGATIVE_MODULE="${2:-}"
      if [ -z "$NEGATIVE_MODULE" ]; then
        echo "错误：--negative-check 需要一个模块名，例如 --negative-check main" >&2
        exit 2
      fi
      shift 2
      ;;
    --arch)
      if [ -z "${2:-}" ] || ! TARGET_ARCH="$(normalize_arch "$2")"; then
        echo "错误：--arch 只接受 arm64 或 x86_64（收到 '${2:-}'）" >&2
        exit 2
      fi
      shift 2
      ;;
    -h|--help)
      sed -n '2,28p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "错误：不认识的参数 '$1'（可用 --arch <arm64|x86_64>、--negative-check <模块> 或 --help）" >&2
      exit 2
      ;;
  esac
done
if [ -z "$TARGET_ARCH" ]; then
  echo "错误：不认识的本机架构 '$(uname -m)'，请用 --arch 指定" >&2
  exit 2
fi

# Per-architecture venv and output directory. The arm64 venv keeps its historical name.
if [ "$TARGET_ARCH" = aarch64 ]; then
  VENV_DIR="$DESKTOP_DIR/.build-venv"
else
  VENV_DIR="$DESKTOP_DIR/.build-venv-$TARGET_ARCH"
fi
OUT_DIR="$DESKTOP_DIR/src-tauri/resources/gateway-$TARGET_ARCH"

# x86_64 on Apple Silicon: re-run the whole script under Rosetta so pip, PyInstaller and the
# smoke check all see an x86_64 process.
if [ "$TARGET_ARCH" != "$NATIVE_ARCH" ]; then
  if [ "$TARGET_ARCH" = x86_64 ] && [ "$(uname -s)" = Darwin ] && [ "$(sysctl -n hw.optional.arm64 2>/dev/null || echo 0)" = 1 ] \
     && [ "${KGW_UNDER_ARCH:-}" != 1 ]; then
    export KGW_UNDER_ARCH=1
    exec arch -x86_64 "${BASH_SOURCE[0]}" "${ORIG_ARGS[@]}"
  fi
  echo "错误：在 $NATIVE_ARCH 机器上无法构建 $TARGET_ARCH 的 sidecar（需要 Apple Silicon 的 Rosetta，或直接在目标架构的机器上构建）" >&2
  exit 1
fi

log() { printf '==> %s\n' "$*"; }

# --- 1. bootstrap interpreter -------------------------------------------------
pick_python() {
  local candidate
  if [ -n "${KGW_PYTHON:-}" ]; then
    candidates=("$KGW_PYTHON")
  elif [ "$TARGET_ARCH" != "$(normalize_arch "$(uname -m)")" ] || [ "${KGW_UNDER_ARCH:-}" = 1 ]; then
    # Cross-architecture build: the default python3 is the wrong architecture; ask uv for one.
    candidates=()
    if command -v uv >/dev/null 2>&1; then
      uv_py="$(uv python find "cpython-3.13-macos-$TARGET_ARCH" 2>/dev/null || true)"
      if [ -n "$uv_py" ]; then candidates=("$uv_py"); fi
    fi
  else
    candidates=(python3.13 python3.12 python3.11 python3.10 python3)
  fi
  for candidate in ${candidates[@]+"${candidates[@]}"}; do
    if command -v "$candidate" >/dev/null 2>&1 \
       && "$candidate" -c 'import sys; sys.exit(0 if sys.version_info >= (3, 10) else 1)' 2>/dev/null; then
      command -v "$candidate"
      return 0
    fi
  done
  return 1
}

if ! BOOT_PYTHON="$(pick_python)"; then
  echo "错误：找不到 $TARGET_ARCH 架构的 Python 3.10 或更新的版本。装一个（例如 brew install python@3.13；x86_64 用 uv python install cpython-3.13-macos-x86_64），或用 KGW_PYTHON 指定路径。" >&2
  exit 1
fi
PY_PLATFORM="$("$BOOT_PYTHON" -c 'import sysconfig; print(sysconfig.get_platform())')"
case "$TARGET_ARCH:$PY_PLATFORM" in
  aarch64:*arm64*|x86_64:*x86_64*) ;;
  *)
    echo "错误：解释器 $BOOT_PYTHON 的平台是 $PY_PLATFORM，与 --arch $TARGET_ARCH 不符。用 KGW_PYTHON 指向 $TARGET_ARCH 版本的 Python。" >&2
    exit 1
    ;;
esac
log "bootstrap Python: $BOOT_PYTHON ($("$BOOT_PYTHON" -c 'import sys, sysconfig; print(sys.version.split()[0], sysconfig.get_platform())'))"

# --- 2. build venv ------------------------------------------------------------
if [ ! -x "$VENV_DIR/bin/python" ]; then
  log "创建构建 venv: $VENV_DIR"
  "$BOOT_PYTHON" -m venv "$VENV_DIR"
fi
VENV_PY="$VENV_DIR/bin/python"

HELPERS="$SIDECAR_SCRIPTS/build_helpers.py"
PYI_VERSION="$("$BOOT_PYTHON" -I "$HELPERS" pyinstaller-version)"
REQ_FILE="$(mktemp -t kgw-req.XXXXXX)"
WORK_DIR="$(mktemp -d -t kgw-sidecar-build.XXXXXX)"
cleanup() { rm -rf "$WORK_DIR" "$REQ_FILE"; }
trap cleanup EXIT

"$BOOT_PYTHON" -I "$HELPERS" requirements "$REPO_ROOT/requirements.txt" > "$REQ_FILE"
log "安装生产依赖和 pyinstaller==$PYI_VERSION"
"$VENV_PY" -m pip install --quiet --disable-pip-version-check -r "$REQ_FILE" "pyinstaller==$PYI_VERSION"

# --- 3. tiktoken BPE ----------------------------------------------------------
STAGE_DIR="$WORK_DIR/stage"
mkdir -p "$STAGE_DIR"
log "下载 tiktoken cl100k_base（缓存文件名 = sha1(url)）"
"$VENV_PY" -I "$HELPERS" download-tiktoken "$STAGE_DIR/tiktoken_cache"

# --- 4. PyInstaller -----------------------------------------------------------
OMIT_ARGS=()
if [ -n "$NEGATIVE_MODULE" ]; then OMIT_ARGS=(--omit "$NEGATIVE_MODULE"); fi
ARGS_FILE="$WORK_DIR/pyinstaller-args.txt"
"$VENV_PY" -I "$HELPERS" pyinstaller-args "$SIDECAR_SCRIPTS/hidden-imports.txt" ${OMIT_ARGS[@]+"${OMIT_ARGS[@]}"} > "$ARGS_FILE"
PYI_ARGS=()
while IFS= read -r arg; do PYI_ARGS+=("$arg"); done < "$ARGS_FILE"

log "PyInstaller --onedir ${NEGATIVE_MODULE:+(去掉 $NEGATIVE_MODULE) }..."
(
  cd "$WORK_DIR"
  "$VENV_PY" -m PyInstaller \
    --noconfirm --clean --onedir --console \
    --name "$APP_NAME" \
    --paths "$REPO_ROOT" \
    --distpath "$WORK_DIR/dist" \
    --workpath "$WORK_DIR/build" \
    --specpath "$WORK_DIR" \
    --exclude-module tkinter \
    "${PYI_ARGS[@]}" \
    "$REPO_ROOT/main.py"
) > "$WORK_DIR/pyinstaller.log" 2>&1 || {
  tail -40 "$WORK_DIR/pyinstaller.log" >&2
  echo "错误：PyInstaller 失败，完整日志见上" >&2
  exit 1
}

# Assemble: executable + _internal from PyInstaller, tiktoken_cache from step 3.
cp -R "$WORK_DIR/dist/$APP_NAME/." "$STAGE_DIR/"
if [ ! -x "$STAGE_DIR/$APP_NAME" ]; then
  echo "错误：构建完成但 $STAGE_DIR/$APP_NAME 不存在或不可执行" >&2
  exit 1
fi

# --- 5. smoke -----------------------------------------------------------------
log "冒烟测试 [$TARGET_ARCH]（临时 HOME、81xx 端口、假 kiro-cli 数据库、不联网）"
set +e
"$VENV_PY" -I "$SIDECAR_SCRIPTS/smoke_sidecar.py" "$STAGE_DIR/$APP_NAME" "$STAGE_DIR/tiktoken_cache" --timeout "$SMOKE_TIMEOUT"
SMOKE_STATUS=$?
set -e

if [ -n "$NEGATIVE_MODULE" ]; then
  if [ "$SMOKE_STATUS" -ne 0 ]; then
    log "反向验证通过：去掉 $NEGATIVE_MODULE 后冒烟如预期失败（输出目录没有被改动）"
    exit 0
  fi
  echo "错误：去掉 $NEGATIVE_MODULE 后冒烟仍然通过，说明冒烟测试挡不住缺失的 hidden-import" >&2
  exit 1
fi

if [ "$SMOKE_STATUS" -ne 0 ]; then
  echo "错误：冒烟测试失败，输出目录保持原样" >&2
  exit 1
fi

# --- 6. publish ---------------------------------------------------------------
log "发布到 $OUT_DIR"
mkdir -p "$OUT_DIR"
find "$OUT_DIR" -mindepth 1 -maxdepth 1 ! -name .gitkeep -exec rm -rf {} +
cp -R "$STAGE_DIR/." "$OUT_DIR/"
touch "$OUT_DIR/.gitkeep"
log "完成：$OUT_DIR/$APP_NAME ($(du -sh "$OUT_DIR" | cut -f1))"
