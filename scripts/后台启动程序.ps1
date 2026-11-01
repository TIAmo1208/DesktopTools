# 后台启动程序并把输出重定向到本次验收产物目录，供用户前台操作、Agent 事后读日志取证。
# 只做「启动 + 日志落地 + 保留进程」，不开启任何调试端口，不改程序行为。
param(
    [string]$Theme = "真机取证",
    [int]$TimeoutSeconds = 120,
    [switch]$KeepRunning
)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot
$Exe = Join-Path $Root "build\target\debug\desktop-tools.exe"
$Stamp = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH-mm-ss")
$OutDir = Join-Path $Root "build\validation\$Theme\$Stamp"
$OutLog = Join-Path $OutDir "stdout.log"
$ErrLog = Join-Path $OutDir "stderr.log"

if (-not (Test-Path $Exe)) {
    Write-Host "[环境] 未找到 Debug 可执行文件：$Exe"
    Write-Host "[环境] 请先执行 cargo build --manifest-path src-tauri/Cargo.toml --bin desktop-tools"
    exit 2
}

New-Item -ItemType Directory -Force $OutDir | Out-Null
New-Item -ItemType Directory -Force (Join-Path $Root "build\tmp") | Out-Null
$env:TMP = Join-Path $Root "build\tmp"

Write-Host "[启动] $Exe"
$Process = Start-Process -FilePath $Exe -PassThru -RedirectStandardOutput $OutLog -RedirectStandardError $ErrLog

# 有界等待：程序要么在窗口内进入运行态，要么已经退出；两者都可作为本次取证的结论。
$Deadline = (Get-Date).AddSeconds($TimeoutSeconds)
while ((Get-Date) -lt $Deadline) {
    Start-Sleep -Milliseconds 500
    if ($Process.HasExited) {
        Write-Host "[退出] 程序已退出，退出码=$($Process.ExitCode)"
        Write-Host "[产物] $OutDir"
        exit 0
    }
    try { if ($Process.MainWindowHandle -ne 0) { break } } catch {}
}

Write-Host "[就绪] PID=$($Process.Id)，程序仍在运行"
Write-Host "[产物] $OutDir"
Write-Host "[日志] 程序日志另见 %LOCALAPPDATA%\TIAmo\DesktopTools\<日期>.log"

if ($KeepRunning) { exit 0 }

Write-Host ""
Write-Host "=== 请在桌面上完成本次要验证的操作，完成后回到本窗口按回车 ==="
Read-Host "按回车收集结果" | Out-Null

$Exited = $Process.HasExited
$ExitCode = if ($Exited) { $Process.ExitCode } else { "运行中" }
Write-Host "[结果] 进程退出=$Exited，退出码=$ExitCode"
Write-Host "[产物] $OutDir"
exit 0
