param(
    [switch]$RemoveSettings
)

$ErrorActionPreference = 'Stop'

if (Get-Process -Name 'sc2-copilot' -ErrorAction SilentlyContinue) {
    throw '请先从托盘退出 SC2 Copilot，再运行卸载脚本。'
}

$localPrograms = [IO.Path]::GetFullPath((Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'Programs'))
$installDirectory = [IO.Path]::GetFullPath((Join-Path $localPrograms 'SC2 Copilot'))
$allowedPrefix = $localPrograms.TrimEnd([IO.Path]::DirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
if (-not $installDirectory.StartsWith($allowedPrefix, [StringComparison]::OrdinalIgnoreCase)) {
    throw "拒绝删除意外路径：$installDirectory"
}

$shortcutPath = Join-Path ([Environment]::GetFolderPath('Programs')) 'SC2 Copilot.lnk'
if (Test-Path -LiteralPath $shortcutPath) {
    Remove-Item -LiteralPath $shortcutPath -Force
}
if (Test-Path -LiteralPath $installDirectory) {
    Remove-Item -LiteralPath $installDirectory -Recurse -Force
}

if ($RemoveSettings) {
    $roamingRoot = [IO.Path]::GetFullPath([Environment]::GetFolderPath('ApplicationData'))
    $settingsDirectory = [IO.Path]::GetFullPath((Join-Path $roamingRoot 'SC2 Copilot'))
    $settingsPrefix = $roamingRoot.TrimEnd([IO.Path]::DirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
    if (-not $settingsDirectory.StartsWith($settingsPrefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "拒绝删除意外设置路径：$settingsDirectory"
    }
    if (Test-Path -LiteralPath $settingsDirectory) {
        Remove-Item -LiteralPath $settingsDirectory -Recurse -Force
    }
}

Write-Host 'SC2 Copilot 已卸载。默认保留用户设置；使用 -RemoveSettings 可一并删除。'
