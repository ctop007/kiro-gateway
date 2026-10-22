<#
.SYNOPSIS
  Build the kiro-gateway sidecar (PyInstaller --onedir) into
  desktop\src-tauri\resources\gateway\ and smoke-test it. Windows twin of build-sidecar.sh.

.DESCRIPTION
  Same steps as the shell script: build venv, production requirements + pinned
  PyInstaller, tiktoken cl100k_base download (cache file named sha1(url)),
  PyInstaller --onedir driven by sidecar\hidden-imports.txt, smoke check, publish.
  The output directory is replaced only after the smoke check passes.

.PARAMETER NegativeCheck
  Module name to leave out of the PyInstaller arguments. The build goes to a
  temp dir and the script succeeds only if the smoke check FAILS.

.NOTES
  KGW_PYTHON         interpreter used to create the venv (default: py -3.13 .. -3.10, python)
  KGW_SMOKE_TIMEOUT  seconds the smoke check waits for /health (default 60)
  Works on Windows PowerShell 5.1 and PowerShell 7. This file is UTF-8 with BOM
  on purpose: 5.1 reads BOM-less files as the ANSI code page.
#>
[CmdletBinding()]
param(
    [string]$NegativeCheck = ""
)

# Native tools write progress to stderr; with 'Stop' Windows PowerShell 5.1 would
# turn that into a terminating error. Exit codes are checked explicitly instead.
$ErrorActionPreference = 'Continue'
Set-StrictMode -Version 2.0

$ScriptDir      = Split-Path -Parent $MyInvocation.MyCommand.Path
$DesktopDir     = Split-Path -Parent $ScriptDir
$RepoRoot       = Split-Path -Parent $DesktopDir
$SidecarScripts = Join-Path $ScriptDir 'sidecar'
$Helpers        = Join-Path $SidecarScripts 'build_helpers.py'
$VenvDir        = Join-Path $DesktopDir '.build-venv'
$OutDir         = Join-Path $DesktopDir 'src-tauri\resources\gateway'
$AppName        = 'kiro-gateway'
$SmokeTimeout   = if ($env:KGW_SMOKE_TIMEOUT) { $env:KGW_SMOKE_TIMEOUT } else { '60' }

function Write-Step([string]$Message) { Write-Host "==> $Message" }

function Stop-Build([string]$Message) {
    [Console]::Error.WriteLine($Message)
    exit 1
}

function Test-PythonOk([string]$Exe, [string[]]$PrefixArgs) {
    try {
        & $Exe @PrefixArgs -c "import sys; sys.exit(0 if sys.version_info >= (3, 10) else 1)" 2>$null
        return ($LASTEXITCODE -eq 0)
    } catch {
        return $false
    }
}

# --- 1. bootstrap interpreter -------------------------------------------------
$BootExe = $null
$BootPrefix = @()
if ($env:KGW_PYTHON) {
    if (Test-PythonOk $env:KGW_PYTHON @()) { $BootExe = $env:KGW_PYTHON }
} else {
    foreach ($minor in '3.13', '3.12', '3.11', '3.10') {
        if ((Get-Command py -ErrorAction SilentlyContinue) -and (Test-PythonOk 'py' @("-$minor"))) {
            $BootExe = 'py'; $BootPrefix = @("-$minor"); break
        }
    }
    if (-not $BootExe -and (Get-Command python -ErrorAction SilentlyContinue) -and (Test-PythonOk 'python' @())) {
        $BootExe = 'python'
    }
}
if (-not $BootExe) {
    Stop-Build "错误：找不到 Python 3.10 或更新的版本。装一个（https://www.python.org/downloads/），或用 KGW_PYTHON 指定路径。"
}
Write-Step "bootstrap Python: $BootExe $($BootPrefix -join ' ')"

# --- 2. build venv ------------------------------------------------------------
$VenvPy = Join-Path $VenvDir 'Scripts\python.exe'
if (-not (Test-Path $VenvPy)) {
    Write-Step "创建构建 venv: $VenvDir"
    & $BootExe @BootPrefix -m venv $VenvDir
    if ($LASTEXITCODE -ne 0) { Stop-Build "错误：创建 venv 失败" }
}

$WorkDir = Join-Path ([IO.Path]::GetTempPath()) ("kgw-sidecar-build-" + [Guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory -Path $WorkDir | Out-Null

try {
    $PyiVersion = (& $BootExe @BootPrefix -I $Helpers pyinstaller-version | Select-Object -First 1)
    if ($LASTEXITCODE -ne 0 -or -not $PyiVersion) { Stop-Build "错误：读取 PyInstaller 版本失败" }

    $Requirements = @(& $BootExe @BootPrefix -I $Helpers requirements (Join-Path $RepoRoot 'requirements.txt'))
    if ($LASTEXITCODE -ne 0 -or $Requirements.Count -eq 0) { Stop-Build "错误：读取生产依赖失败" }

    Write-Step "安装生产依赖和 pyinstaller==$PyiVersion"
    & $VenvPy -m pip install --quiet --disable-pip-version-check @Requirements "pyinstaller==$PyiVersion"
    if ($LASTEXITCODE -ne 0) { Stop-Build "错误：pip install 失败" }

    # --- 3. tiktoken BPE ------------------------------------------------------
    $StageDir = Join-Path $WorkDir 'stage'
    New-Item -ItemType Directory -Path $StageDir | Out-Null
    Write-Step "下载 tiktoken cl100k_base（缓存文件名 = sha1(url)）"
    & $VenvPy -I $Helpers download-tiktoken (Join-Path $StageDir 'tiktoken_cache')
    if ($LASTEXITCODE -ne 0) { Stop-Build "错误：tiktoken 缓存准备失败" }

    # --- 4. PyInstaller -------------------------------------------------------
    $OmitArgs = @()
    if ($NegativeCheck) { $OmitArgs = @('--omit', $NegativeCheck) }
    $PyiArgs = @(& $VenvPy -I $Helpers pyinstaller-args (Join-Path $SidecarScripts 'hidden-imports.txt') @OmitArgs)
    if ($LASTEXITCODE -ne 0) { Stop-Build "错误：生成 PyInstaller 参数失败" }

    $suffix = if ($NegativeCheck) { "(去掉 $NegativeCheck) " } else { "" }
    Write-Step "PyInstaller --onedir $suffix..."
    $PyiLog = Join-Path $WorkDir 'pyinstaller.log'
    Push-Location $WorkDir
    try {
        & $VenvPy -m PyInstaller `
            --noconfirm --clean --onedir --console `
            --name $AppName `
            --paths $RepoRoot `
            --distpath (Join-Path $WorkDir 'dist') `
            --workpath (Join-Path $WorkDir 'build') `
            --specpath $WorkDir `
            --exclude-module tkinter `
            @PyiArgs `
            (Join-Path $RepoRoot 'main.py') *> $PyiLog
        $PyiStatus = $LASTEXITCODE
    } finally {
        Pop-Location
    }
    if ($PyiStatus -ne 0) {
        Get-Content $PyiLog -Tail 40 | ForEach-Object { [Console]::Error.WriteLine($_) }
        Stop-Build "错误：PyInstaller 失败，完整日志见上"
    }

    # Assemble: executable + _internal from PyInstaller, tiktoken_cache from step 3.
    Copy-Item -Path (Join-Path $WorkDir "dist\$AppName\*") -Destination $StageDir -Recurse -Force
    $StageExe = Join-Path $StageDir "$AppName.exe"
    if (-not (Test-Path $StageExe)) { Stop-Build "错误：构建完成但 $StageExe 不存在" }

    # --- 5. smoke -------------------------------------------------------------
    Write-Step "冒烟测试（临时 HOME、81xx 端口、假 kiro-cli 数据库、不联网）"
    & $VenvPy -I (Join-Path $SidecarScripts 'smoke_sidecar.py') $StageExe (Join-Path $StageDir 'tiktoken_cache') --timeout $SmokeTimeout
    $SmokeStatus = $LASTEXITCODE

    if ($NegativeCheck) {
        if ($SmokeStatus -ne 0) {
            Write-Step "反向验证通过：去掉 $NegativeCheck 后冒烟如预期失败（输出目录没有被改动）"
            exit 0
        }
        Stop-Build "错误：去掉 $NegativeCheck 后冒烟仍然通过，说明冒烟测试挡不住缺失的 hidden-import"
    }
    if ($SmokeStatus -ne 0) { Stop-Build "错误：冒烟测试失败，输出目录保持原样" }

    # --- 6. publish -----------------------------------------------------------
    Write-Step "发布到 $OutDir"
    New-Item -ItemType Directory -Path $OutDir -Force | Out-Null
    Get-ChildItem -LiteralPath $OutDir -Force | Where-Object { $_.Name -ne '.gitkeep' } | Remove-Item -Recurse -Force
    Copy-Item -Path (Join-Path $StageDir '*') -Destination $OutDir -Recurse -Force
    New-Item -ItemType File -Path (Join-Path $OutDir '.gitkeep') -Force | Out-Null
    $SizeMb = [math]::Round(((Get-ChildItem -LiteralPath $OutDir -Recurse -File | Measure-Object Length -Sum).Sum) / 1MB, 1)
    Write-Step "完成：$OutDir\$AppName.exe ($SizeMb MB)"
} finally {
    Remove-Item -LiteralPath $WorkDir -Recurse -Force -ErrorAction SilentlyContinue
}
