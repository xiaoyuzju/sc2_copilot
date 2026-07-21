# SC2 Copilot

SC2 Copilot 是面向《星际争霸 II》合作任务的只读辅助程序。它使用 Rust 独立实现，只读取游戏在本机 `127.0.0.1:6119` 暴露的状态，不截取游戏画面、不读取游戏内存、不修改游戏文件，也不模拟玩家输入。

首版包含 15 张合作任务地图的灰机 Wiki 时间表、确定性计时与去重引擎、6119 状态适配器、透明置顶覆盖层、地图/分支/阶段锚点的手动兜底、托盘、设置、诊断和可替换的提醒播放接口。具体提示音与全部视觉识别能力尚未实现，详见 [延期功能](docs/deferred-features.md)。

## 开发运行

环境要求：Windows 11、PowerShell 7、Rust 1.97 或兼容的更新 stable 工具链。

```powershell
cargo run --release -p sc2-copilot-app --bin sc2-copilot
```

没有启动 SC2 时程序仍可运行，设置窗口会显示 6119 未连接。关闭设置窗口不会退出程序；请使用托盘菜单重新打开设置或退出。

全量验证：

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
```

## 离线 6119 回放

回放工具按顺序读取若干组 `/game/` 与 `/ui/` JSON，不访问网络：

```powershell
cargo run -p sc2-copilot-app --bin sc2-replay -- `
  crates/sc2-copilot-app/tests/fixtures/sc2/oblivion-game.json `
  crates/sc2-copilot-app/tests/fixtures/sc2/in-game-ui.json
```

开发期也可以启动无窗口的实时脱敏监控。它不会保存玩家名称或完整端点响应，只记录连接/菜单/对局状态、内部会话号、识别出的地图 ID、游戏时间、玩家数量与诊断；对局中最多每个游戏秒写一条：

```powershell
cargo run -p sc2-copilot-app --bin sc2-monitor -- logs/sc2-monitor.jsonl
```

## 发布包

```powershell
pwsh -File scripts/package-release.ps1
```

脚本只把两个已编译的可执行文件、安装/卸载脚本和用户说明复制到 `dist/`。开发期的灰机 Wiki 原始快照、Keiframe 差异报告及数据库不会进入发布包。

数据更新流程见 [重复抓取手册](docs/runbooks/update-map-schedule-data.md)，总体实现边界见 [首版范围](docs/initial-release-scope.md)。
