# SC2 Copilot

SC2 Copilot 是面向《星际争霸 II》合作任务的只读辅助程序。它使用 Rust 独立实现，直连游戏在本机 `127.0.0.1:6119` 暴露的状态且不继承系统代理；只在目标地图的短检测时间窗内截取 SC2 客户区中的小地图区域，不读取游戏内存、不修改游戏文件，也不模拟玩家输入。画面只在内存中处理，默认不保存截图。

首版包含 15 张合作任务地图的灰机 Wiki 时间表、确定性计时与去重引擎、6119 状态适配器、透明置顶覆盖层、地图/分支/突变因子/阶段锚点的手动兜底、托盘、设置、诊断和可替换的提醒播放接口。具体提示音尚未实现。小地图红点自动分支目前只用于“往日神庙”和“虚空撕裂”：仅在 SC2 位于前台、客户区为不小于 1280×720 的无黑边 16:9 布局且 UI 缩放为 100% 时运行；1080p、1440p、4K 坐标已有测试，HDR 捕获路径已完成实时采集冒烟。虚空撕裂的真实无红点路径已验证，真实目标区域红点和往日神庙仍需继续标定。详见 [视觉方案](docs/vision-design.md)。

## 开发运行

环境要求：Windows 11、PowerShell 7、Rust 1.97 或兼容的更新 stable 工具链。

```powershell
cargo run --release -p sc2-copilot-app --bin sc2-copilot
```

没有启动 SC2 时程序仍可运行，设置窗口会显示 6119 未连接。关闭设置窗口不会退出程序；请使用托盘菜单重新打开设置或退出。

## 隐私与诊断

- 默认只在内存中处理小地图 ROI，不保存游戏截图。
- 不把玩家名称、账号标识、聊天内容或完整 6119 响应写入日志、诊断文件或文档；不读取游戏内存。
- 实时监控仅记录连接状态、单次进程内的临时会话序号、地图 ID、游戏时间、玩家数量和脱敏诊断。
- 实战标定文档只保留聚合后的技术结论，不记录玩家信息、本机绝对路径或可识别设备的详细元数据。
- 日志和显式导出的裁剪 ROI 只用于本地诊断；分享前应由操作者自行复核内容。

全量验证：

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
```

## 覆盖层交互

在设置页点击“录制热键”，再直接按下需要的组合键；按 `Esc` 取消录制，“清除”会注销当前热键。进入对局后，用该热键或托盘菜单启用交互模式，即可拖动标题栏和窗口边框调整覆盖层位置与大小；再次按热键或 `Esc` 恢复鼠标穿透。位置与大小会自动保存。

## 离线 6119 回放

回放工具按顺序读取若干组 `/game/` 与 `/ui/` JSON，不访问网络：

```powershell
cargo run -p sc2-copilot-app --bin sc2-fixture-replay -- `
  crates/sc2-copilot-app/tests/fixtures/sc2/oblivion-game.json `
  crates/sc2-copilot-app/tests/fixtures/sc2/in-game-ui.json
```

开发期也可以启动无窗口的实时脱敏监控。它遵守上面的隐私边界；对局中最多每个游戏秒写一条：

```powershell
cargo run -p sc2-copilot-app --bin sc2-monitor -- logs/sc2-monitor.jsonl
```

## 发布包

```powershell
pwsh -File scripts/package-release.ps1
```

脚本只把两个已编译的可执行文件、安装/卸载脚本和用户说明复制到 `dist/`。开发期的灰机 Wiki 原始快照、Keiframe 差异报告及数据库不会进入发布包。

验证发布包的精确内容；在确认当前用户没有安装或运行 SC2 Copilot、SC2 未运行后，可追加当前 Windows 环境的安装、启动和卸载冒烟测试。干净 Windows 11 与断网环境仍需单独验收：

```powershell
pwsh -File scripts/verify-release.ps1
pwsh -File scripts/verify-release.ps1 -InstallSmokeTest
```

数据更新流程见 [重复抓取手册](docs/runbooks/update-map-schedule-data.md)，总体实现边界见 [首版范围](.scratch/sc2-copilot/spec.md)。
