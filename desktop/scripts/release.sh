#!/usr/bin/env bash
# Release the desktop app: bump versions, build, sign, upload to Tencent COS.
#
#   desktop/scripts/release.sh VERSION=0.0.2 NOTES=notes.md [FORCE=1] [--dry-run] [--skip-build]
#
# VERSION  x.y.z, must be higher than the current version.
# FIRST_RELEASE=1  nothing is published yet, so there is no live latest.json to read.
#          Without it a non-forced release aborts when the live manifest cannot be
#          read (it would otherwise lower minRequiredVersion back to 0.0.1).
# NOTES    path of a release-notes file, or one line of text (one note per line).
# FORCE=1  minRequiredVersion = VERSION: every older install must upgrade. Asks
#          you to type the version again.
# --dry-run    No real upload and no changes to the three version files: prints
#              the upload plan and writes the manifest to the staging directory.
# --skip-build use existing build outputs (with --dry-run and none present, fake
#              placeholder outputs are generated so the manifest can be checked).
#
# Upload order: installers -> update packages -> .sig -> HEAD check of every
# file -> latest.json last, so the manifest never points at missing files.
#
# Signing key: ~/.kiro-gateway/desktop-updater.key (no password), or
# TAURI_SIGNING_PRIVATE_KEY_PATH. COS credentials: COS_SECRET_ID / COS_SECRET_KEY
# or ~/.tccli/default.credential; they are never printed.
set -euo pipefail

DESKTOP="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LIB="$DESKTOP/scripts/release_lib.py"
PYTHON="${RELEASE_PYTHON:-/opt/miniconda3/bin/python}"
export PATH="$HOME/.cargo/bin:$PATH"

VERSION="${VERSION:-}"
NOTES="${NOTES:-}"
FORCE="${FORCE:-0}"
FIRST_RELEASE="${FIRST_RELEASE:-0}"
DRY_RUN=0
SKIP_BUILD=0
STAGING="${RELEASE_STAGING:-}"

for arg in "$@"; do
  case "$arg" in
    VERSION=*) VERSION="${arg#VERSION=}" ;;
    NOTES=*) NOTES="${arg#NOTES=}" ;;
    FORCE=*) FORCE="${arg#FORCE=}" ;;
    FIRST_RELEASE=*) FIRST_RELEASE="${arg#FIRST_RELEASE=}" ;;
    --dry-run) DRY_RUN=1 ;;
    --skip-build) SKIP_BUILD=1 ;;
    -h|--help) sed -n '2,25p' "$0"; exit 0 ;;
    *) echo "未知参数：$arg（见 --help）" >&2; exit 2 ;;
  esac
done

py() { "$PYTHON" -I "$LIB" "$@"; }

[ -n "$VERSION" ] || { echo "缺少 VERSION=x.y.z" >&2; exit 2; }
[ -n "$NOTES" ] || { echo "缺少 NOTES=<文件或一行说明>" >&2; exit 2; }
case "$FORCE" in 0|1) ;; *) echo "FORCE 只能是 0 或 1" >&2; exit 2 ;; esac
case "$FIRST_RELEASE" in 0|1) ;; *) echo "FIRST_RELEASE 只能是 0 或 1" >&2; exit 2 ;; esac

# Fails early on a bad or non-increasing version; writes nothing.
py check-version --root "$DESKTOP" --version "$VERSION"

if [ "$FORCE" = "1" ]; then
  if ! py confirm-force --version "$VERSION"; then
    echo >&2
    echo "版本号不一致，已取消。" >&2
    exit 1
  fi
fi

# A real release must be able to read the live latest.json (it carries the floor
# minRequiredVersion forward). Checked before anything is written.
if [ "$DRY_RUN" = "0" ]; then
  CHECK=(check-published)
  [ "$FORCE" = "1" ] && CHECK+=(--force)
  [ "$FIRST_RELEASE" = "1" ] && CHECK+=(--first-release)
  py "${CHECK[@]}"
fi

[ -n "$STAGING" ] || STAGING="$(mktemp -d "${TMPDIR:-/tmp}/kiro-gateway-release.XXXXXX")"
mkdir -p "$STAGING"
BUNDLE="$DESKTOP/src-tauri/target/universal-apple-darwin/release/bundle"
FAKE_BUNDLE="$STAGING/fake-bundle"

# The version bump is a rollback-able step: package.json, tauri.conf.json, Cargo.toml and
# Cargo.lock are snapshotted first and restored byte for byte on any failure or interrupt
# before the release is complete. Otherwise the tree would stay on the new version and the
# rerun would be refused ("must be higher than the current version").
SNAPSHOT=""
RELEASE_DONE=0
rollback_versions() {
  local status=$?
  trap - EXIT
  if [ -n "$SNAPSHOT" ] && [ "$RELEASE_DONE" = "0" ]; then
    echo "发布未完成，恢复版本号文件到发布前的内容。" >&2
    py restore --root "$DESKTOP" --dir "$SNAPSHOT" >&2 || echo "恢复失败，请手动检查：$SNAPSHOT" >&2
  fi
  exit "$status"
}
trap rollback_versions EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

if [ "$DRY_RUN" = "0" ]; then
  SNAPSHOT="$(mktemp -d "${TMPDIR:-/tmp}/kiro-gateway-release-versions.XXXXXX")"
  py snapshot --root "$DESKTOP" --dir "$SNAPSHOT"
  py bump --root "$DESKTOP" --version "$VERSION"
fi

if [ "$SKIP_BUILD" = "0" ]; then
  if [ "$DRY_RUN" = "1" ]; then
    echo "[dry-run] 跳过出包（加 --skip-build 也可使用已有产物）。使用占位产物生成清单。"
    py fake-artifacts --version "$VERSION" --bundle-dir "$FAKE_BUNDLE"
    BUNDLE="$FAKE_BUNDLE"
  else
    KEY="${TAURI_SIGNING_PRIVATE_KEY_PATH:-$HOME/.kiro-gateway/desktop-updater.key}"
    [ -f "$KEY" ] || { echo "找不到签名私钥：$KEY" >&2; exit 1; }
    # The Tauri 2 CLI reads TAURI_SIGNING_PRIVATE_KEY (the key text or a file path);
    # it ignores the *_PATH variant. Pass the path so the key text never enters the environment dump.
    export TAURI_SIGNING_PRIVATE_KEY="$KEY"
    export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}"
    (cd "$DESKTOP" && pnpm tauri build --target universal-apple-darwin --bundles app,dmg \
      --config '{"bundle":{"createUpdaterArtifacts":true}}')
  fi
else
  if [ "$DRY_RUN" = "1" ] && [ ! -d "$BUNDLE" ]; then
    echo "[dry-run] 没有已有产物，使用占位产物。"
    py fake-artifacts --version "$VERSION" --bundle-dir "$FAKE_BUNDLE"
    BUNDLE="$FAKE_BUNDLE"
  fi
fi

PREPARE=(prepare --version "$VERSION" --notes "$NOTES" --bundle-dir "$BUNDLE" --out "$STAGING/out")
[ "$FORCE" = "1" ] && PREPARE+=(--force)
[ "$DRY_RUN" = "1" ] && PREPARE+=(--no-fetch)
[ "$FIRST_RELEASE" = "1" ] && PREPARE+=(--first-release)
[ -n "${WINDOWS_BUNDLE_DIR:-}" ] && PREPARE+=(--windows-dir "$WINDOWS_BUNDLE_DIR")
py "${PREPARE[@]}"
py verify-manifest "$STAGING/out/latest.json"

if [ "$DRY_RUN" = "1" ]; then
  echo "[dry-run] 没有上传，也没有修改版本号。清单：$STAGING/out/latest.json"
  exit 0
fi

py upload --staging "$STAGING/out"
RELEASE_DONE=1
[ -n "$SNAPSHOT" ] && rm -rf "$SNAPSHOT"
echo "发布完成：$VERSION"
