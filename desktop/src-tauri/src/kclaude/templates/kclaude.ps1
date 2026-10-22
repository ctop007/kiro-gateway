# kclaude: run Claude Code through the local Kiro Gateway.
# Managed by the Kiro Gateway App. Edits are overwritten the next time the App starts.
# This file is saved as UTF-8 with a BOM so Windows PowerShell 5 reads the messages correctly.
#
# Every run reads kclaude.env from the App data directory, so changing the port, access
# key or model mapping in the App takes effect in sessions that are already open.
function kclaude {
    $kcDir = @@KCLAUDE_DATA_DIR@@
    $kcEnv = Join-Path $kcDir 'kclaude.env'

    function Stop-Kclaude([string]$Message) {
        [Console]::Error.WriteLine("kclaude: $Message")
        $global:LASTEXITCODE = 1
    }

    if (-not (Test-Path -LiteralPath $kcEnv -PathType Leaf)) {
        Stop-Kclaude '找不到 Kiro Gateway 配置，请打开 Kiro Gateway App'
        return
    }

    # kclaude.env is a shell-style KEY=value file; read the keys we need without executing it.
    $cfg = @{}
    foreach ($line in [System.IO.File]::ReadAllLines($kcEnv)) {
        if ($line -match '^\s*(?:export\s+)?([A-Z][A-Z0-9_]*)=(.*)$') {
            $value = $Matches[2].Trim()
            if ($value.Length -ge 2 -and (($value.StartsWith("'") -and $value.EndsWith("'")) -or ($value.StartsWith('"') -and $value.EndsWith('"')))) {
                $value = $value.Substring(1, $value.Length - 2)
            }
            $cfg[$Matches[1]] = $value
        }
    }

    if ($cfg['KIRO_GATEWAY_UPGRADE_REQUIRED'] -eq '1') {
        Stop-Kclaude 'Kiro Gateway 需要升级，请打开 App 完成升级'
        return
    }

    $port = $cfg['KIRO_GATEWAY_PORT']
    $key = $cfg['KIRO_GATEWAY_KEY']
    if (-not $port -or $port -notmatch '^[0-9]+$' -or -not $key) {
        Stop-Kclaude '找不到 Kiro Gateway 配置，请打开 Kiro Gateway App'
        return
    }

    # Probe with a proxy-free HttpClient: the REST cmdlets on PS5 follows the system proxy
    # (Clash and similar), which cannot reach 127.0.0.1 and would report a false "not running".
    $healthy = $false
    $client = $null
    try {
        Add-Type -AssemblyName System.Net.Http -ErrorAction SilentlyContinue
        $handler = New-Object System.Net.Http.HttpClientHandler
        $handler.UseProxy = $false
        $client = New-Object System.Net.Http.HttpClient($handler)
        $client.Timeout = [TimeSpan]::FromSeconds(2)
        $resp = $client.GetAsync("http://127.0.0.1:$port/health").GetAwaiter().GetResult()
        $healthy = $resp.IsSuccessStatusCode
    } catch {
        $healthy = $false
    } finally {
        if ($client) { $client.Dispose() }
    }
    if (-not $healthy) {
        Stop-Kclaude 'KiroGateway 未运行，请在 Kiro Gateway App 里打开开关'
        return
    }

    if (-not (Get-Command claude -ErrorAction SilentlyContinue)) {
        Stop-Kclaude '找不到 claude 命令，请先安装 Claude Code'
        return
    }

    $opus = if ($cfg['KCLAUDE_OPUS_MODEL']) { $cfg['KCLAUDE_OPUS_MODEL'] } else { 'claude-opus-5-5[1m]' }
    $sonnet = if ($cfg['KCLAUDE_SONNET_MODEL']) { $cfg['KCLAUDE_SONNET_MODEL'] } else { 'claude-sonnet-5-5[1m]' }
    $haiku = if ($cfg['KCLAUDE_HAIKU_MODEL']) { $cfg['KCLAUDE_HAIKU_MODEL'] } else { 'claude-haiku-4-5' }

    $names = 'ANTHROPIC_BASE_URL', 'ANTHROPIC_AUTH_TOKEN', 'ANTHROPIC_DEFAULT_OPUS_MODEL', 'ANTHROPIC_DEFAULT_SONNET_MODEL', 'ANTHROPIC_DEFAULT_HAIKU_MODEL', 'NO_PROXY', 'no_proxy'
    $saved = @{}
    foreach ($n in $names) { $saved[$n] = [Environment]::GetEnvironmentVariable($n, 'Process') }
    try {
        [Environment]::SetEnvironmentVariable('ANTHROPIC_BASE_URL', "http://127.0.0.1:$port", 'Process')
        [Environment]::SetEnvironmentVariable('ANTHROPIC_AUTH_TOKEN', $key, 'Process')
        [Environment]::SetEnvironmentVariable('ANTHROPIC_DEFAULT_OPUS_MODEL', $opus, 'Process')
        [Environment]::SetEnvironmentVariable('ANTHROPIC_DEFAULT_SONNET_MODEL', $sonnet, 'Process')
        [Environment]::SetEnvironmentVariable('ANTHROPIC_DEFAULT_HAIKU_MODEL', $haiku, 'Process')
        # Keep the user's own entries and make sure claude never proxies the local gateway.
        foreach ($n in 'NO_PROXY', 'no_proxy') {
            $old = $saved[$n]
            $merged = if ($old) { "$old,127.0.0.1,localhost" } else { '127.0.0.1,localhost' }
            [Environment]::SetEnvironmentVariable($n, $merged, 'Process')
        }
        & claude @args
    } finally {
        foreach ($n in $names) { [Environment]::SetEnvironmentVariable($n, $saved[$n], 'Process') }
    }
}
