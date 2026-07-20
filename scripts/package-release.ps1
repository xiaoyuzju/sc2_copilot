param(
    [string]$Version = '0.1.0'
)

$ErrorActionPreference = 'Stop'

$workspace = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$distRoot = [IO.Path]::GetFullPath((Join-Path $workspace 'dist'))
$packageName = "sc2-copilot-$Version-windows-x64"
$packageDirectory = [IO.Path]::GetFullPath((Join-Path $distRoot $packageName))
$allowedPrefix = $distRoot.TrimEnd([IO.Path]::DirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
if (-not $packageDirectory.StartsWith($allowedPrefix, [StringComparison]::OrdinalIgnoreCase)) {
    throw "拒绝清理意外路径：$packageDirectory"
}

Push-Location $workspace
try {
    cargo build --release --locked -p sc2-copilot-app --bins
    if ($LASTEXITCODE -ne 0) {
        throw "cargo build 失败，退出码 $LASTEXITCODE"
    }

    if (Test-Path -LiteralPath $packageDirectory) {
        Remove-Item -LiteralPath $packageDirectory -Recurse -Force
    }
    New-Item -ItemType Directory -Path $packageDirectory -Force | Out-Null

    Copy-Item -LiteralPath (Join-Path $workspace 'target\release\sc2-copilot.exe') -Destination $packageDirectory
    Copy-Item -LiteralPath (Join-Path $workspace 'target\release\sc2-replay.exe') -Destination $packageDirectory
    Copy-Item -LiteralPath (Join-Path $workspace 'packaging\Install.ps1') -Destination $packageDirectory
    Copy-Item -LiteralPath (Join-Path $workspace 'packaging\Uninstall.ps1') -Destination $packageDirectory
    Copy-Item -LiteralPath (Join-Path $workspace 'packaging\README.txt') -Destination $packageDirectory

    $archivePath = "$packageDirectory.zip"
    if (Test-Path -LiteralPath $archivePath) {
        Remove-Item -LiteralPath $archivePath -Force
    }
    Compress-Archive -LiteralPath $packageDirectory -DestinationPath $archivePath
    Write-Host "发布包已生成：$archivePath"
}
finally {
    Pop-Location
}
