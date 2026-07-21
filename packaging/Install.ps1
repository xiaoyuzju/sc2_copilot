$ErrorActionPreference = 'Stop'

$localPrograms = Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'Programs'
$installDirectory = Join-Path $localPrograms 'SC2 Copilot'
$startMenuDirectory = [Environment]::GetFolderPath('Programs')
$shortcutPath = Join-Path $startMenuDirectory 'SC2 Copilot.lnk'

New-Item -ItemType Directory -Path $installDirectory -Force | Out-Null
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'sc2-copilot.exe') -Destination $installDirectory -Force
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'sc2-fixture-replay.exe') -Destination $installDirectory -Force
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'Uninstall.ps1') -Destination $installDirectory -Force

$shell = New-Object -ComObject WScript.Shell
$shortcut = $shell.CreateShortcut($shortcutPath)
$shortcut.TargetPath = Join-Path $installDirectory 'sc2-copilot.exe'
$shortcut.WorkingDirectory = $installDirectory
$shortcut.Description = 'SC2 Copilot'
$shortcut.Save()

Write-Host "SC2 Copilot 已安装到 $installDirectory"
