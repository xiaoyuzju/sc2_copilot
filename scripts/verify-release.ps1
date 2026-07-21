param(
    [string]$Version = '0.1.0',
    [switch]$InstallSmokeTest
)

$ErrorActionPreference = 'Stop'

$workspace = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$packageName = "sc2-copilot-$Version-windows-x64"
$packageDirectory = Join-Path $workspace "dist\$packageName"
$archivePath = "$packageDirectory.zip"
$expectedFiles = @(
    'Install.ps1'
    'README.txt'
    'sc2-copilot.exe'
    'sc2-fixture-replay.exe'
    'Uninstall.ps1'
) | Sort-Object

& (Join-Path $PSScriptRoot 'package-release.ps1') -Version $Version

$actualFiles = @(Get-ChildItem -LiteralPath $packageDirectory -File |
        Select-Object -ExpandProperty Name |
        Sort-Object)
if (Compare-Object $expectedFiles $actualFiles) {
    throw "发布目录内容不符合预期：$($actualFiles -join ', ')"
}

Add-Type -AssemblyName System.IO.Compression.FileSystem
$archive = [IO.Compression.ZipFile]::OpenRead($archivePath)
try {
    $archiveFiles = @($archive.Entries |
            Where-Object { -not [string]::IsNullOrEmpty($_.Name) } |
            Select-Object -ExpandProperty Name |
            Sort-Object)
    if (Compare-Object $expectedFiles $archiveFiles) {
        throw "ZIP 内容不符合预期：$($archiveFiles -join ', ')"
    }
}
finally {
    $archive.Dispose()
}

if (-not $InstallSmokeTest) {
    Write-Host "发布包验证通过：$archivePath"
    exit 0
}

$localPrograms = [IO.Path]::GetFullPath((Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'Programs'))
$installDirectory = [IO.Path]::GetFullPath((Join-Path $localPrograms 'SC2 Copilot'))
$allowedPrefix = $localPrograms.TrimEnd([IO.Path]::DirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
if (-not $installDirectory.StartsWith($allowedPrefix, [StringComparison]::OrdinalIgnoreCase)) {
    throw "拒绝使用意外安装路径：$installDirectory"
}
$shortcutPath = Join-Path ([Environment]::GetFolderPath('Programs')) 'SC2 Copilot.lnk'
if ((Test-Path -LiteralPath $installDirectory) -or
    (Test-Path -LiteralPath $shortcutPath) -or
    (Get-Process -Name 'sc2-copilot' -ErrorAction SilentlyContinue) -or
    (Get-Process -Name 'SC2', 'SC2_x64' -ErrorAction SilentlyContinue)) {
    throw '检测到已有 SC2 Copilot 安装、快捷方式、运行中程序或 SC2 对局；为避免覆盖现有环境并确保无 SC2 启动测试，已跳过安装冒烟测试。'
}

$installedExe = Join-Path $installDirectory 'sc2-copilot.exe'
$uninstaller = Join-Path $installDirectory 'Uninstall.ps1'
$started = $null
try {
    & (Join-Path $packageDirectory 'Install.ps1')
    if (-not (Test-Path -LiteralPath $installedExe -PathType Leaf)) {
        throw '安装后主程序不存在。'
    }
    if (-not (Test-Path -LiteralPath $shortcutPath -PathType Leaf)) {
        throw '安装后开始菜单快捷方式不存在。'
    }

    $started = Start-Process -FilePath $installedExe -WorkingDirectory $installDirectory -WindowStyle Hidden -PassThru
    if (-not $started.WaitForInputIdle(10000)) {
        throw '程序未在十秒内进入可交互状态。'
    }
    Start-Sleep -Milliseconds 750
    $started.Refresh()
    if ($started.HasExited) {
        throw "程序启动后过早退出，退出码 $($started.ExitCode)"
    }
}
finally {
    if ($null -ne $started) {
        $started.Refresh()
    }
    if ($null -ne $started -and -not $started.HasExited) {
        Stop-Process -Id $started.Id -Force
        $started.WaitForExit(10000) | Out-Null
    }
    if (Test-Path -LiteralPath $uninstaller -PathType Leaf) {
        & $uninstaller
    }
    else {
        if (Test-Path -LiteralPath $shortcutPath) {
            Remove-Item -LiteralPath $shortcutPath -Force
        }
        if (Test-Path -LiteralPath $installDirectory) {
            Remove-Item -LiteralPath $installDirectory -Recurse -Force
        }
    }
}

if ((Test-Path -LiteralPath $installDirectory) -or (Test-Path -LiteralPath $shortcutPath)) {
    throw '卸载后仍残留安装目录或快捷方式。'
}

Write-Host '发布包内容、安装、启动和卸载验证全部通过。'
