# kclaude: run Claude Code through the local kiro-gateway.
#
# Usage: add this line to ~/.zshrc (above any block that must stay last):
#   source "$HOME/kiro-gateway/scripts/kclaude.zsh"
#
# Then use `kclaude` exactly like `claude`. Plain `claude` is unaffected.
#
# What it does:
#   1. Reads PROXY_API_KEY from <gateway dir>/.env
#   2. Starts the gateway in the background (127.0.0.1 only) if it is not running
#   3. Runs Claude Code with ANTHROPIC_BASE_URL / ANTHROPIC_AUTH_TOKEN pointing at it,
#      mapping Claude Code's opus / sonnet / haiku tiers to Kiro models
#
# Optional overrides (set before sourcing, or export in the shell):
#   KIRO_GATEWAY_DIR      gateway checkout          (default: $HOME/kiro-gateway)
#   KIRO_GATEWAY_PORT     gateway port              (default: 8000)
#   KIRO_GATEWAY_LOG      gateway log file          (default: $HOME/Library/Logs/kiro-gateway.log)
#   KCLAUDE_OPUS_MODEL    model for the opus tier   (default: claude-opus-5-5[1m])
#   KCLAUDE_SONNET_MODEL  model for the sonnet tier (default: claude-sonnet-5-5[1m])
#   KCLAUDE_HAIKU_MODEL   model for the haiku tier  (default: claude-haiku-4-5)
#
# Model names use the dash form Claude Code recognizes; "[1m]" tells Claude Code the
# model has a 1M context window. The gateway normalizes these to Kiro model IDs.

kclaude() {
  local gw_dir="${KIRO_GATEWAY_DIR:-$HOME/kiro-gateway}"
  local port="${KIRO_GATEWAY_PORT:-8000}"
  local log="${KIRO_GATEWAY_LOG:-$HOME/Library/Logs/kiro-gateway.log}"
  local health="http://127.0.0.1:$port/health"
  local key

  key=$(sed -n -E 's/^PROXY_API_KEY=["'\'']?([^"'\'']*)["'\'']?.*/\1/p' "$gw_dir/.env" 2>/dev/null | head -1)
  if [[ -z "$key" ]]; then
    echo "kclaude: 在 $gw_dir/.env 里找不到 PROXY_API_KEY" >&2
    return 1
  fi

  if ! curl -s -m 2 "$health" >/dev/null; then
    if [[ ! -x "$gw_dir/.venv/bin/python" ]]; then
      echo "kclaude: 找不到 $gw_dir/.venv，请先按 docs/internal/local-setup.md 安装" >&2
      return 1
    fi
    echo "kclaude: 网关未运行，正在启动（日志: $log）..." >&2
    mkdir -p "${log:h}"
    (cd "$gw_dir" && nohup .venv/bin/python main.py --host 127.0.0.1 --port "$port" >>"$log" 2>&1 &!)
    local i
    for i in {1..30}; do
      curl -s -m 1 "$health" >/dev/null && break
      sleep 1
    done
    if ! curl -s -m 2 "$health" >/dev/null; then
      echo "kclaude: 网关启动失败，请查看 $log" >&2
      return 1
    fi
  fi

  ANTHROPIC_BASE_URL="http://127.0.0.1:$port" \
  ANTHROPIC_AUTH_TOKEN="$key" \
  ANTHROPIC_DEFAULT_OPUS_MODEL="${KCLAUDE_OPUS_MODEL:-claude-opus-5-5[1m]}" \
  ANTHROPIC_DEFAULT_SONNET_MODEL="${KCLAUDE_SONNET_MODEL:-claude-sonnet-5-5[1m]}" \
  ANTHROPIC_DEFAULT_HAIKU_MODEL="${KCLAUDE_HAIKU_MODEL:-claude-haiku-4-5}" \
  command claude "$@"
}
