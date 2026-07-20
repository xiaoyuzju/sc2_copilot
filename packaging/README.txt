SC2 Copilot Windows x64
=======================

安装：在 PowerShell 7 中运行
  pwsh -ExecutionPolicy Bypass -File .\Install.ps1

程序只读取 http://127.0.0.1:6119/game/ 与 /ui/，无需外网。
关闭设置窗口后程序仍在托盘运行；请通过托盘退出。

卸载：先从托盘退出，然后运行安装目录中的
  pwsh -ExecutionPolicy Bypass -File .\Uninstall.ps1

默认保留 %APPDATA%\SC2 Copilot\settings.json。若也要删除设置：
  pwsh -ExecutionPolicy Bypass -File .\Uninstall.ps1 -RemoveSettings

提示音提供器与视觉识别功能不在此版本中；覆盖层提醒和播放接口已经保留。
