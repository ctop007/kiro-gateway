#!/usr/bin/env bash
# Publish a desktop release that GitHub Actions has built to Tencent COS.
#
#   desktop/scripts/publish-from-github.sh desktop-v0.0.2 [--dry-run]
#
# The tag's "Desktop Release" run builds the mac universal dmg + update package and
# the Windows NSIS installer, signs them and attaches them to the GitHub Release.
# This script downloads that run's artifacts, builds latest.json and uploads to COS
# in the safe order (installers -> packages -> .sig -> HEAD check -> latest.json),
# reusing release_lib.py. COS credentials never leave this machine.
#
# Release notes = the annotated tag's message body. A line "[force-upgrade]" in it
# makes minRequiredVersion = this version (every older install must upgrade); the
# marker line itself is not shown to users.
#
# FIRST_RELEASE=1  nothing is published on COS yet (no live latest.json to read).
# --dry-run        download + build the manifest + print the upload plan, no upload.
#
# Needs: gh (logged in, access to the repo), COS_SECRET_ID / COS_SECRET_KEY or
# ~/.tccli/default.credential.
set -euo pipefail

REPO="${KGW_GITHUB_REPO:-heyuan110/kiro-gateway}"
WORKFLOW="desktop-release.yml"
DESKTOP="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LIB="$DESKTOP/scripts/release_lib.py"
PYTHON="${RELEASE_PYTHON:-/opt/miniconda3/bin/python}"

TAG=""
DRY_RUN=0
for arg in "$@"; do
  case "$arg" in
    --dry-run) DRY_RUN=1 ;;
    desktop-v*) TAG="$arg" ;;
    *) echo "错误：不认识的参数 '${arg}'（用法：publish-from-github.sh desktop-vX.Y.Z [--dry-run]）" >&2; exit 2 ;;
  esac
done
if [ -z "$TAG" ]; then
  echo "错误：要给出 tag，例如 desktop-v0.0.2" >&2
  exit 2
fi
VERSION="${TAG#desktop-v}"
py() { "$PYTHON" -I "$LIB" "$@"; }

echo "==> 找 ${TAG} 的构建"
RUN_ID="$(gh run list -R "$REPO" --workflow "$WORKFLOW" --branch "$TAG" --limit 1 \
  --json databaseId,status,conclusion \
  -q '.[0] | select(.status == "completed" and .conclusion == "success") | .databaseId')"
if [ -z "$RUN_ID" ]; then
  echo "错误：${TAG} 还没有成功完成的构建。到 https://github.com/${REPO}/actions 看看是否还在跑或失败了。" >&2
  exit 1
fi

STAGING="$(mktemp -d "${TMPDIR:-/tmp}/kgw-publish.XXXXXX")"
trap 'rm -rf "$STAGING"' EXIT
echo "==> 下载构建 ${RUN_ID} 的产物"
gh run download "$RUN_ID" -R "$REPO" -D "$STAGING/artifacts"
MAC_BUNDLE="$STAGING/artifacts/bundles-macOS/universal-apple-darwin/release/bundle"
WIN_BUNDLE="$STAGING/artifacts/bundles-Windows/release/bundle"

echo "==> 读 tag 说明作为更新说明"
git -C "$DESKTOP" fetch --quiet internal "+refs/tags/${TAG}:refs/tags/${TAG}" 2>/dev/null || true
if [ "$(git -C "$DESKTOP" cat-file -t "$TAG" 2>/dev/null)" != "tag" ]; then
  echo "错误：${TAG} 不是带说明的 tag（用 git tag -a ${TAG} -F notes.md 打）" >&2
  exit 1
fi
NOTES_RAW="$(git -C "$DESKTOP" tag -l --format='%(contents:body)' "$TAG")"
[ -n "$NOTES_RAW" ] || NOTES_RAW="$(git -C "$DESKTOP" tag -l --format='%(contents:subject)' "$TAG")"
FORCE_ARGS=()
if printf '%s\n' "$NOTES_RAW" | grep -qx '\[force-upgrade\]'; then
  FORCE_ARGS=(--force)
  echo "⚠️  强制升级：minRequiredVersion = ${VERSION}"
fi
printf '%s\n' "$NOTES_RAW" | grep -vx '\[force-upgrade\]' > "$STAGING/notes.md" || true

PREPARE=(prepare --version "$VERSION" --notes "$STAGING/notes.md" --out "$STAGING/out"
  --bundle-dir "$MAC_BUNDLE" --windows-dir "$WIN_BUNDLE")
[ ${#FORCE_ARGS[@]} -gt 0 ] && PREPARE+=("${FORCE_ARGS[@]}")
[ "${FIRST_RELEASE:-0}" = 1 ] && PREPARE+=(--first-release)
py "${PREPARE[@]}"
py verify-manifest "$STAGING/out/latest.json"

if [ "$DRY_RUN" = 1 ]; then
  echo "==> --dry-run：不上传。清单："
  cat "$STAGING/out/latest.json"
  exit 0
fi
echo "==> 上传到 COS"
py upload --staging "$STAGING/out"
echo "==> 已发布 ${VERSION}：https://tools-1254376798.cos.ap-guangzhou.myqcloud.com/kiro-gateway/desktop/latest.json"
