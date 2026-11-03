# 按窗口截图：先试 PrintWindow(PW_RENDERFULLCONTENT)，拿不到内容就报告失败。
# 实机屏幕截图会抢占用户桌面，必须由用户同意后显式带 -实机屏幕截图 -已获用户同意 才执行。
param(
    [string]$进程名 = "desktop-tools",
    [string]$输出 = "",
    [switch]$实机屏幕截图,
    [switch]$已获用户同意
)

$错误前缀 = "[环境]"

# 解析目标窗口：取该进程第一个可见且有标题的顶层窗口。
$进程 = Get-Process -Name $进程名 -ErrorAction SilentlyContinue | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
if (-not $进程) {
    Write-Output "$错误前缀 没有找到进程 $进程名 的可见窗口"
    exit 2
}

$窗口句柄 = $进程.MainWindowHandle
$窗口标题 = $进程.MainWindowTitle

Add-Type -AssemblyName System.Drawing

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class 取证窗口 {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint flags);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
}
"@

# 最小化窗口无法渲染，先还原再处理。
if ([取证窗口]::IsIconic($窗口句柄)) { [取证窗口]::ShowWindow($窗口句柄, 9) | Out-Null; Start-Sleep -Milliseconds 400 }

$矩形 = New-Object 取证窗口+RECT
[取证窗口]::GetWindowRect($窗口句柄, [ref]$矩形) | Out-Null
$宽 = $矩形.Right - $矩形.Left
$高 = $矩形.Bottom - $矩形.Top
if ($宽 -le 0 -or $高 -le 0) {
    Write-Output "$错误前缀 窗口尺寸无效：${宽}x${高}"
    exit 2
}

# 未指定输出路径时，产物落在 build/validation 下，不入库。
if ([string]::IsNullOrWhiteSpace($输出)) {
    $时间戳 = Get-Date -Format "yyyyMMdd-HHmmss"
    $根 = Join-Path $PSScriptRoot "..\build\validation\窗口截图"
    New-Item -ItemType Directory -Force $根 | Out-Null
    $输出 = Join-Path $根 "$进程名-$时间戳.png"
}
New-Item -ItemType Directory -Force (Split-Path -Parent $输出) | Out-Null

# 首选 PrintWindow：PW_RENDERFULLCONTENT=0x2，能取到 DirectComposition 内容。
$位图 = New-Object System.Drawing.Bitmap $宽, $高
$画布 = [System.Drawing.Graphics]::FromImage($位图)
$设备上下文 = $画布.GetHdc()
$打印成功 = [取证窗口]::PrintWindow($窗口句柄, $设备上下文, 2)
$画布.ReleaseHdc($设备上下文)
$画布.Dispose()

# PrintWindow 成功也可能得到全黑图：抽查四角与中心是否有非零像素。
$有内容 = $false
if ($打印成功) {
    foreach ($点 in @(@(2, 2), @(($宽 - 3), 2), @(2, ($高 - 3)), @(($宽 - 3), ($高 - 3)), @([int]($宽 / 2), [int]($高 / 2)))) {
        $像素 = $位图.GetPixel($点[0], $点[1])
        if ($像素.R -ne 0 -or $像素.G -ne 0 -or $像素.B -ne 0) { $有内容 = $true; break }
    }
}

if ($有内容) {
    $位图.Save($输出, [System.Drawing.Imaging.ImageFormat]::Png)
    $位图.Dispose()
    Write-Output "[窗口抓取] $输出"
    Write-Output "[窗口] $窗口标题 ${宽}x${高}"
    exit 0
}
$位图.Dispose()

# PrintWindow 拿不到内容时，只有用户明确同意才允许退到实机屏幕截图。
if (-not $实机屏幕截图) {
    Write-Output "$错误前缀 PrintWindow 未取到可辨识内容（可能是透明或 DirectComposition 窗口）"
    Write-Output "$错误前缀 需要实机屏幕截图时，先停下来告知用户并取得同意，再加 -实机屏幕截图 -已获用户同意"
    exit 2
}
if (-not $已获用户同意) {
    Write-Output "$错误前缀 实机屏幕截图会抢占桌面操作，必须由用户明确同意"
    exit 2
}

[取证窗口]::SetForegroundWindow($窗口句柄) | Out-Null
Start-Sleep -Milliseconds 400
$屏幕位图 = New-Object System.Drawing.Bitmap $宽, $高
$屏幕画布 = [System.Drawing.Graphics]::FromImage($屏幕位图)
$屏幕画布.CopyFromScreen($矩形.Left, $矩形.Top, 0, 0, $屏幕位图.Size)
$屏幕画布.Dispose()
$屏幕位图.Save($输出, [System.Drawing.Imaging.ImageFormat]::Png)
$屏幕位图.Dispose()
Write-Output "[实机屏幕截图] $输出"
Write-Output "[窗口] $窗口标题 ${宽}x${高}"
exit 0
