# kclaude: run Claude Code through the local Kiro Gateway.
# Managed by the Kiro Gateway App. Edits are overwritten the next time the App starts.
#
# Every run reads kclaude.env from the App data directory, so changing the port, access
# key or model mapping in the App takes effect in terminals that are already open.
# The function runs in a subshell: nothing it sets leaks into your shell, and the plain
# `claude` command is never touched.
kclaude() {
  emulate -L zsh
  local kc_dir=@@KCLAUDE_DATA_DIR@@
  local kc_env="$kc_dir/kclaude.env"

  if [ ! -r "$kc_env" ]; then
    echo "kclaude: 找不到 Kiro Gateway 配置，请打开 Kiro Gateway App" >&2
    return 1
  fi

  (
    unset KIRO_GATEWAY_PORT KIRO_GATEWAY_KEY KCLAUDE_OPUS_MODEL KCLAUDE_SONNET_MODEL \
          KCLAUDE_HAIKU_MODEL KIRO_GATEWAY_UPGRADE_REQUIRED
    . "$kc_env" || {
      echo "kclaude: 找不到 Kiro Gateway 配置，请打开 Kiro Gateway App" >&2
      exit 1
    }

    if [ "${KIRO_GATEWAY_UPGRADE_REQUIRED:-}" = "1" ]; then
      echo "kclaude: Kiro Gateway 需要升级，请打开 App 完成升级" >&2
      exit 1
    fi

    case "${KIRO_GATEWAY_PORT:-}" in
      '' | *[!0-9]*)
        echo "kclaude: 找不到 Kiro Gateway 配置，请打开 Kiro Gateway App" >&2
        exit 1
        ;;
    esac
    if [ -z "${KIRO_GATEWAY_KEY:-}" ]; then
      echo "kclaude: 找不到 Kiro Gateway 配置，请打开 Kiro Gateway App" >&2
      exit 1
    fi

    # A proxy exported in the user's shell (Clash on 127.0.0.1:7890, ...) must not intercept
    # the loopback probe, and must not intercept claude's own calls to the local gateway.
    NO_PROXY="${NO_PROXY:+$NO_PROXY,}127.0.0.1,localhost"
    no_proxy="${no_proxy:+$no_proxy,}127.0.0.1,localhost"
    export NO_PROXY no_proxy

    if ! curl -sf -m 2 --noproxy '*' "http://127.0.0.1:${KIRO_GATEWAY_PORT}/health" >/dev/null 2>&1; then
      echo "kclaude: KiroGateway 未运行，请在 Kiro Gateway App 里打开开关" >&2
      exit 1
    fi

    if ! command -v claude >/dev/null 2>&1; then
      echo "kclaude: 找不到 claude 命令，请先安装 Claude Code" >&2
      exit 127
    fi

    ANTHROPIC_BASE_URL="http://127.0.0.1:${KIRO_GATEWAY_PORT}" \
    ANTHROPIC_AUTH_TOKEN="$KIRO_GATEWAY_KEY" \
    ANTHROPIC_DEFAULT_OPUS_MODEL="${KCLAUDE_OPUS_MODEL:-claude-opus-5-5[1m]}" \
    ANTHROPIC_DEFAULT_SONNET_MODEL="${KCLAUDE_SONNET_MODEL:-claude-sonnet-5-5[1m]}" \
    ANTHROPIC_DEFAULT_HAIKU_MODEL="${KCLAUDE_HAIKU_MODEL:-claude-haiku-4-5}" \
    command claude "$@"
  )
}
