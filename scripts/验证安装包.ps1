# 在独立中文空格目录验证首次安装、覆盖安装与卸载；不启动桌面宿主。
param([Parameter(Mandatory = $true)][string]$Installer)

$ErrorActionPreference = 'Stop'
$installerPath = (Resolve-Path -LiteralPath $Installer).Path # 本次待验证安装包。
$root = Split-Path -Parent $PSScriptRoot # 仓库根目录。
$directory = Join-Path $root 'build/安装包 验证' # 固定测试目录，卸载只处理安装器拥有的文件。
$uninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Desktop Tools' # 产品安装登记。
$menuKey = 'HKCU:\Software\Classes\DesktopBackground\shell\DesktopTools' # 需要恢复的原桌面菜单。
$manufacturerKey = 'HKCU:\Software\Desktop Tools Project\Desktop Tools' # 安装器保存的路径与语言。
$backup = Join-Path $root 'build/installer-menu-backup.reg' # 原桌面菜单快照。
$manufacturerBackup = Join-Path $root 'build/installer-manufacturer-backup.reg' # 原安装器偏好快照。

if (Test-Path -LiteralPath $uninstallKey) { throw '已存在正式安装登记，拒绝覆盖用户安装；请在独立测试环境运行。' }
if (Test-Path -LiteralPath $directory) { throw '测试目录已存在，请先检查上次验收结果并清理后再运行。' }
if (Get-Process desktop-tools,desktop-tools-guardian -ErrorAction SilentlyContinue) { throw '请先正常退出 Desktop Tools 再验证。' }

$hadMenu = Test-Path -LiteralPath $menuKey # 保留测试前菜单是否存在的事实。
$hadManufacturer = Test-Path -LiteralPath $manufacturerKey # 保留测试前安装器偏好。
if ($hadMenu) {
    & reg.exe export 'HKCU\Software\Classes\DesktopBackground\shell\DesktopTools' $backup /y | Out-Null
    if ($LASTEXITCODE -ne 0) { throw '备份原桌面菜单失败。' }
}
if ($hadManufacturer) {
    & reg.exe export 'HKCU\Software\Desktop Tools Project\Desktop Tools' $manufacturerBackup /y | Out-Null
    if ($LASTEXITCODE -ne 0) { throw '备份原安装器偏好失败。' }
}

try {
    # 重复安装同一版本，验证安装钩子与两个二进制始终一致。
    foreach ($round in 1..2) {
        $process = Start-Process -FilePath $installerPath -ArgumentList "/S /D=$directory" -WindowStyle Hidden -PassThru # 安装过程。
        if (!$process.WaitForExit(60000)) { throw '安装超时；保留现场，不强制结束进程。' }
        if ($process.ExitCode -ne 0) { throw "安装失败，退出码：$($process.ExitCode)" }
        foreach ($binary in 'desktop-tools.exe','desktop-tools-guardian.exe') {
            $actual = (Get-FileHash -LiteralPath (Join-Path $directory $binary)).Hash # 安装后的文件哈希。
            $expected = (Get-FileHash -LiteralPath (Join-Path $root "build/target/release/$binary")).Hash # 同次构建的文件哈希。
            if ($binary -eq 'desktop-tools.exe') {
                # Tauri 仅在打包副本中把 UNK 标记改为 NSS，比较预期补丁后的完整哈希。
                $expected = & node -e 'const fs=require("fs"),crypto=require("crypto"),assert=require("assert/strict");const b=fs.readFileSync(process.argv[1]),m=Buffer.from("__TAURI_BUNDLE_TYPE_VAR_UNK"),i=b.indexOf(m);assert(i>=0 && b.indexOf(m,i+1)===-1,"打包标记必须唯一");b.write("NSS",i+m.length-3);console.log(crypto.createHash("sha256").update(b).digest("hex").toUpperCase());' (Join-Path $root "build/target/release/$binary")
                if ($LASTEXITCODE -ne 0) { throw '无法校验 Tauri 安装包标记。' }
            }
            if ($actual -ne $expected) { throw "安装文件与当前构建不一致：$binary" }
        }
        if (!(Test-Path -LiteralPath (Join-Path $directory '许可证/第三方许可证.txt'))) { throw '安装包缺少第三方许可声明。' }
        $command = (Get-Item -LiteralPath "$menuKey\command").GetValue('') # 安装后退出态菜单指向本次安装位置。
        if (!$command.Contains($directory)) { throw '桌面菜单没有指向测试安装目录。' }
        Write-Output "INFO 第 $round 次安装通过：主程序、guardian、许可证与菜单路径正确。"
    }

    # 原地执行卸载器避免复制到临时目录后父进程提前退出。
    $process = Start-Process -FilePath (Join-Path $directory 'uninstall.exe') -ArgumentList "/S _?=$directory" -WindowStyle Hidden -PassThru # 同步观察的卸载进程。
    if (!$process.WaitForExit(60000)) { throw '卸载超时；保留现场，不强制结束进程。' }
    if ($process.ExitCode -ne 0) { throw "卸载失败，退出码：$($process.ExitCode)" }
    if ((Test-Path -LiteralPath $uninstallKey) -or (Test-Path -LiteralPath $menuKey) -or (Test-Path -LiteralPath (Join-Path $directory 'desktop-tools.exe')) -or (Test-Path -LiteralPath (Join-Path $directory 'desktop-tools-guardian.exe'))) { throw '卸载后仍有程序或注册表残留。' }
    Write-Output 'INFO 卸载验证通过；测试未启动桌面宿主，不代表运行中升级与恢复已验收。'
}
finally {
    # 成功卸载后恢复测试前状态；失败时保留现场，避免掩盖未卸载程序。
    if (!(Test-Path -LiteralPath $uninstallKey)) {
        if ($hadMenu) {
            & reg.exe import $backup | Out-Null
            if ($LASTEXITCODE -ne 0) { throw '恢复原桌面菜单失败。' }
        }
        if ($hadManufacturer) {
            & reg.exe import $manufacturerBackup | Out-Null
            if ($LASTEXITCODE -ne 0) { throw '恢复原安装器偏好失败。' }
        }
        elseif (Test-Path -LiteralPath $manufacturerKey) { Remove-Item -LiteralPath $manufacturerKey -Recurse }
    }
}
