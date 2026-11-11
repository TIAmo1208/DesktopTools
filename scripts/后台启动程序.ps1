# 后台启动程序并把输出重定向到本次验收产物目录，供用户前台操作、Agent 事后读日志取证。
# 只做「启动 + 日志落地 + 默认保留进程」，不开启任何调试端口，不改程序行为。
param(
    [string]$Theme = "真机取证",
    [int]$TimeoutSeconds = 120,
    [switch]$KeepRunning, # 兼容旧调用：默认即保留进程。
    [switch]$WaitForExit  # 需要脚本内等待退出时显式传入；非交互回合不要用。
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
    Write-Host "[环境] 请先执行 npm run tauri:dev 或 cargo build --manifest-path src-tauri/Cargo.toml --bin desktop-tools"
    exit 2
}

# 已安装实例占用桌面菜单互斥体，开发版会静默退出；先查进程，避免把 exit 0 当成启动成功。
$已有 = @(Get-Process desktop-tools, desktop-tools-guardian -ErrorAction SilentlyContinue)
if ($已有.Count -gt 0) {
    Write-Host "[环境] 已存在 Desktop Tools 实例：PID=$($已有.Id -join ',')"
    Write-Host "[环境] 它占用 Local\DesktopTools.DesktopMenu.Instance，开发版启动后会静默退出（exit 0）。"
    Write-Host "[环境] 请先正常退出已安装实例，再重新运行本脚本。"
    exit 3
}

New-Item -ItemType Directory -Force $OutDir | Out-Null
New-Item -ItemType Directory -Force (Join-Path $Root "build\tmp") | Out-Null
$env:TMP = Join-Path $Root "build\tmp"

Write-Host "[启动] $Exe"
$Process = Start-Process -FilePath $Exe -PassThru -RedirectStandardOutput $OutLog -RedirectStandardError $ErrLog

# 有界等待：程序要么在窗口内进入运行态，要么已经退出；两者都作为本次结论，不重试。
$Deadline = (Get-Date).AddSeconds($TimeoutSeconds)
$就绪 = $false
while ((Get-Date) -lt $Deadline) {
    Start-Sleep -Milliseconds 500
    if ($Process.HasExited) {
        $退出码 = $Process.ExitCode
        Write-Host "[退出] 进程在窗口就绪前退出，退出码=$退出码"
        if ($退出码 -eq 0) {
            Write-Host "[诊断] 退出码 0 且未出现窗口，常见原因是互斥体被占用或初始化提前结束；不要据此认为程序已启动。"
            Write-Host "[诊断] 请查看 $ErrLog 与 %LOCALAPPDATA%\TIAmo\DesktopTools\<日期>.log。"
        }
        Write-Host "[产物] $OutDir"
        exit 1
    }
    try { if ($Process.MainWindowHandle -ne 0) { $就绪 = $true; break } } catch {}
}

if (-not $就绪) {
    Write-Host "[环境] 在 $TimeoutSeconds 秒内未等到窗口；进程仍在运行（PID=$($Process.Id)），按未就绪处理。"
    Write-Host "[产物] $OutDir"
    exit 1
}

Write-Host "[就绪] PID=$($Process.Id)，程序仍在运行"
Write-Host "[产物] $OutDir"
Write-Host "[日志] 程序日志另见 %LOCALAPPDATA%\TIAmo\DesktopTools\<日期>.log"

# 默认直接返回，进程留给用户前台操作；需要脚本内等待时才阻塞。
if ($WaitForExit) {
    Write-Host "[等待] 等待进程退出；结束前台操作后进程关闭即返回。"
    $Process.WaitForExit()
    Write-Host "[结果] 进程已退出，退出码=$($Process.ExitCode)"
}

exit 0
