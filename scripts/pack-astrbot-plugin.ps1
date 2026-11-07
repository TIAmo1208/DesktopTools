# Pack the AstrBot status collector as a zip installable from the AstrBot Web UI.
# Repeatable: the previous archive is overwritten. ASCII-only so Windows PowerShell 5.1
# reads it the same way regardless of the active code page (no BOM needed).
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.IO.Compression # ZipArchive and compression level.
Add-Type -AssemblyName System.IO.Compression.FileSystem
$root = Split-Path -Parent $PSScriptRoot # Repository root.
# Do not hard-code the localized folder name; find the folder under plugin/ that holds the package.
$pluginRoot = Get-ChildItem -LiteralPath (Join-Path $root 'plugin') -Directory | Where-Object { Test-Path -LiteralPath (Join-Path $_.FullName 'astrbot_plugin_desktop_status') } | Select-Object -First 1
if (!$pluginRoot) { throw 'astrbot_plugin_desktop_status source directory not found under plugin/.' }
$pluginDirectory = Join-Path $pluginRoot.FullName 'astrbot_plugin_desktop_status' # Plugin source directory.
$archivePath = Join-Path $pluginRoot.FullName 'astrbot_plugin_desktop_status.zip' # Distribution artifact.
$packageName = 'astrbot_plugin_desktop_status' # Root directory inside the archive.
$files = @('main.py', 'metadata.yaml', '_conf_schema.json', 'README.md', 'snapshot.schema.json') # Runtime files only; excludes __pycache__ and .pyc by construction.

# Write entries directly instead of Compress-Archive: Windows PowerShell 5.1 emits backslash
# separators, which AstrBot's unzip may not interpret as directories.
$stream = [System.IO.File]::Open($archivePath, [System.IO.FileMode]::Create)
$zip = New-Object System.IO.Compression.ZipArchive($stream, [System.IO.Compression.ZipArchiveMode]::Create)
try {
    foreach ($file in $files) {
        $source = Join-Path $pluginDirectory $file
        if (!(Test-Path -LiteralPath $source -PathType Leaf)) { throw "Missing plugin file: $file" }
        $entry = $zip.CreateEntry("$packageName/$file", [System.IO.Compression.CompressionLevel]::Optimal) # Forward slashes keep the required root directory.
        $entryStream = $entry.Open()
        try {
            $bytes = [System.IO.File]::ReadAllBytes($source)
            $entryStream.Write($bytes, 0, $bytes.Length)
        } finally {
            $entryStream.Dispose()
        }
    }
} finally {
    $zip.Dispose()
    $stream.Dispose()
}

$archive = Get-Item -LiteralPath $archivePath
Write-Output ("INFO packed {0} ({1} bytes)" -f $archive.FullName, $archive.Length)
$readStream = [System.IO.File]::OpenRead($archive.FullName)
$readZip = New-Object System.IO.Compression.ZipArchive($readStream, [System.IO.Compression.ZipArchiveMode]::Read)
try {
    foreach ($entry in $readZip.Entries) {
        if ($entry.FullName -match '__pycache__|\.pyc$') { throw "Forbidden file in archive: $($entry.FullName)" }
        Write-Output ("INFO entry {0} ({1} bytes)" -f $entry.FullName, $entry.Length)
    }
} finally {
    $readZip.Dispose()
    $readStream.Dispose()
}
