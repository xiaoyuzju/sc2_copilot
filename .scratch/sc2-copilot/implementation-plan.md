# SC2 Copilot Implementation Plan

本计划是首版 Rust 重实现的执行基线。它只参考 Keiframe 与游戏交互相关的可观察行为和固定数据库差异，不采用其源码、数据库设计、提示文案、简写、音频或视觉参数。

## Success criteria

- Windows 11 上以原生 Rust 程序运行，不依赖 Python、WebView 或 JavaScript 运行时。
- 只读取 SC2 本机 6119 HTTP 状态和玩家在 SC2 Copilot 中的显式操作，不注入、不读取游戏内存、不修改游戏文件、不模拟输入。
- 覆盖灰机 Wiki 总览中的全部 15 张基础地图。
- 15 张地图的每张相关表都必须有可追溯处理结果：`automatic`、`manual_context` 或带原因的 `unsupported`，不能静默丢弃。
- 灰机 Wiki 是发布值基线；Keiframe 固定 commit `192bdbce6868e597b297cf47f485ac5c79eb9baf` 只产生差异报告。
- 审核后的规范化 JSON 在构建时校验并嵌入可执行文件；运行时不访问灰机 Wiki、不读取外部时间表。
- 暂不实现任何视觉识别；延期能力完整记录在 [Deferred Features](../../docs/deferred-features.md)。

## Workspace shape

使用一个小型 Cargo workspace，避免把开发期抓取/SQLite 依赖带进玩家程序：

```text
crates/
  sc2-copilot-core/   # 纯 Rust 领域模型、目录校验和时间线引擎
  sc2-copilot-app/    # eframe/egui、6119、Win32、设置和可执行程序
tools/
  schedule-data/      # 开发期规范化、校验和 Keiframe 差异检查
data/
  sources/huiji/      # 原始 DOM 快照，仅审查
  maps/               # 审核后的规范化 JSON，构建输入
  diffs/              # Keiframe 差异报告，仅审查
```

不继续拆分更多 crate。crate 内部以深 module 隐藏复杂度，只有真正存在生产和测试两种 adapter 的地方才建立 port。

## Deep modules and seams

### `sc2-copilot-core::CopilotEngine`

这是首版最重要的深 module。它的 interface 接收确定性输入并返回结果，不直接执行 I/O：

```text
apply(EngineInput) -> EngineUpdate
```

`EngineInput` 覆盖游戏状态观测、玩家命令、设置变化和显式会话迁移。`EngineUpdate` 返回覆盖层视图、同刻提醒批次和诊断。它的 implementation 隐藏：

- 会话识别与结束清理；
- 地图/分支上下文；
- 开局时钟和手动阶段锚点；
- `automatic` / `manual_context` / `unsupported` 过滤；
- 提前窗口、同刻批次、`notified` / `missed` 去重；
- 暂停、前跳、倒退、重连和新局状态迁移。

调用者和测试只通过这个 interface 观察行为，不直接测试内部计时器或集合。

### `sc2-copilot-core::ScheduleCatalog`

interface 只负责从调用者提供的 JSON 字节构造一个不可变目录，并按地图/变体查询；应用层负责用 `include_bytes!` 提供内嵌字节：

```text
from_json(bytes) -> Result<ScheduleCatalog>
schedule_for(map_id, variant_context) -> ScheduleView
```

implementation 隐藏 JSON 反序列化、schema 版本、稳定 ID、来源引用、触发器校验和全地图覆盖检查。应用层不能访问 raw DOM 表格。

### Game-state source seam

应用层定义最小轮询 interface，生产 adapter 读取 `127.0.0.1:6119`，测试 adapter 回放录制夹具。轮询 implementation 运行在独立后台线程，用容量为 1 的“最新状态”通道交付观测；消费者落后时覆盖旧状态，不积压历史响应，也不引入异步运行时。

### Alert-audio seam

提醒交付层调用已经决定的非阻塞播放 interface。首版提供无操作 adapter 和测试记录 adapter；具体提示音、TTS 和播放库延期决定。失败只产生诊断，不回滚弹窗或事件去重。

### Windows shell adapters

eframe/egui 实现设置窗口、覆盖层提示卡片和临时交互面板。Win32 adapter 只补充置顶、透明鼠标穿透、全局热键和必要的窗口行为。业务时钟、去重和地图判断不能放进 UI 回调或 Win32 消息处理。

### `schedule-data`

开发期深 module 的 interface 是：

```text
normalize(snapshot_batch) -> CompileReport
validate(compiled_catalog) -> ValidationReport
diff_keiframe(compiled_catalog, fixed_reference) -> DiffReport
```

implementation 内部包含按表头签名匹配的显式 table adapter、合并单元格展开、主表/附表连接、重复汇总去重和 15 地图覆盖矩阵。Keiframe adapter 只读固定 `maps.db`，不能反向写入规范化目录。

## Data model

运行时采用 [Map Schedule Design](../../docs/map-schedule-design.md) 已确认的结构：

```text
CompiledEvent {
  map_id
  variant_id?
  event_id
  trigger: Trigger
  facts: Fact[]
  source_refs: SourceRef[]
  runtime_support
}
```

- 首版 `Trigger`：`AtGameTime`、`AtGameTimeWindow`、`AtStageElapsed`、`AtStageRemaining`。条件、依赖、组合和重复表达式只有在来源关系经过独立建模后才加入；当前对应来源行显式记为 `unsupported`，见 [Deferred Features](../../docs/deferred-features.md)。
- `Fact`：事件类别、波次、位置、路线、目标、规模、科技、生命/护盾、数量/组成、概率、备选组、经白名单确认的补充字段，以及由用户手动启用的突变上下文。未知事实列会使编译失败，不静默丢弃。
- `SourceRef`：来源 URL、快照批次、相对路径、表索引及逻辑单元格引用。
- `runtime_support`：`automatic`、`manual_context`、`unsupported`。

未知表头、未处理字段、无法确认的时钟锚点或逻辑关系会使数据编译失败或产生必须人工处置的 `unsupported` 记录，不能自动猜测。

## Delivery phases

### Phase 1 — Workspace and quality gates

建立三个 crate、格式化、静态检查、测试和日志基线。保持现有功能为空，不先搭建通用插件系统或配置框架。

验证：

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`

### Phase 2 — Snapshot validation and compiled schema

读取现有 2026-07-21 快照，验证 15 地图、149 表、1,609 行、URL、连续表索引和合并单元格；实现 `Trigger`、`Fact`、`SourceRef`、运行支持状态与 schema 版本。

先用“湮灭快车”建立端到端 tracer bullet，但它只证明管线，不代表地图范围完成。

验证：同一原始行能追溯到编译事件；25:00 同时/分支事件不被压扁；损坏 schema、重复事件 ID 或缺失来源引用会失败。

### Phase 3 — All-map table adapters

按复杂度逐批完成，但首版必须全部交付：

1. 直接绝对时间：湮灭快车、黑暗杀星、聚铁成兵。
2. 多表目标/进攻：虚空降临、天界封锁、升格之链。
3. 分支、公式和时间窗口：虚空撕裂、往日神庙、机会渺茫、熔火危机、克哈裂痕。
4. 阶段与条件：营救矿工、亡者之夜、死亡摇篮、净网行动。

每张相关表必须进入覆盖清单：成功编译，或带来源和原因成为 `unsupported`。净网行动的 OCR 倒计时事件保留但不运行。

验证：生成逐地图覆盖报告，相关表的 `handled + unsupported = total`，且没有未分类行。

### Phase 4 — Keiframe difference checker

只读查询固定 `maps.db`，生成覆盖、时间、多事件、数值、精度损失、结构压缩和词汇差异。比较器不自动补数据，也不复制提示文本或简写。

验证：湮灭快车复现现有 19/19 时间点结果；三组 15→18 配置映射明确；冲突仍保留灰机 Wiki 发布值。

### Phase 5 — Pure timeline engine

实现会话、地图上下文、时钟、手动锚点、提醒提前量、同刻批次、错过策略和去重。所有时间推进通过输入数据，不调用系统时钟。

验证：使用表驱动测试覆盖正常推进、暂停、前跳、倒退、重连、新局、锚点创建/替换/清除、同刻批次和所有提醒状态。

### Phase 6 — SC2 6119 adapter and fixture replay

实现只读 HTTP 轮询、响应规范化、连接状态和地图自动识别。保存脱敏的本地夹具用于回放测试，不在运行时持久化完整对局遥测。

验证：未启动游戏、菜单、进入对局、同局暂断、退出和新局都产生确定的引擎输入；端点异常不会阻塞 UI。

### Phase 7 — Windows product shell

实现设置窗口、托盘、透明置顶覆盖层、鼠标穿透、临时交互模式、手动锚点按钮和热键诊断。具体默认热键仍是待定设置，不写死进领域模型。

验证：交互模式只影响本应用；Esc/热键恢复穿透；离局关闭面板；注册失败有后备入口；覆盖层不抢占游戏焦点。

### Phase 8 — Alert delivery

实现覆盖层提示卡片、默认 30 秒全局提前量、同刻合并、一次性去重、恢复时立即提醒或标记错过，以及提醒播放 port 的无操作 adapter。

验证：不创建 Windows 系统通知；不同事件时间不误合并；播放失败不重复弹窗或改变去重；到点不二次请求播放。

### Phase 9 — Settings, diagnostics, packaging

只持久化跨局设置，例如提醒提前量、覆盖层位置和未来确定的热键；锚点、已通知和错过事件只存在当前会话。诊断展示 6119 连接、当前会话、地图/分支、数据快照版本、unsupported 原因、热键和播放 port 状态。

验证：干净 Windows 11 环境可安装和卸载；无 SC2、无网络、无音频提供器时仍可启动；发布包不包含 raw Wiki 快照、Keiframe 数据库或差异报告。

## Test strategy

- 核心引擎只做确定性输入/输出测试，不 mock 私有 implementation。
- 6119 使用录制夹具 adapter；播放使用记录/no-op adapter；设置使用临时目录。
- 15 张地图数据使用固定快照 golden tests，重点覆盖合并单元格、同秒事件、分支、窗口、公式和阶段锚点。
- UI 只测试窗口状态和视图模型转换；时间线逻辑不在 egui 测试中重复。
- 每个已修复的数据错误都留下最小回归夹具。

## Explicitly deferred

- 所有游戏画面采集与视觉识别；
- 净网行动 OCR 动态提醒；
- 敌方种族和地图分支视觉自动选择；
- 人口、神器、小地图 ping 等识别；
- 具体提示音、TTS、音频库和默认热键；
- Keiframe 界面、Excel 导入导出、提示文案、音频和风暴英雄编码。

## Known risks

- 灰机 Wiki 是社区二手来源；项目只保证与选定快照一致。
- 部分页面的章节标题和时钟锚点没有进入 raw 表格，相关条目必须人工确认或保持 `unsupported`。
- 15 地图全覆盖的数据工作量明显高于运行时引擎本身，应先完成覆盖报告再宣称功能完成。
- 全局热键、透明穿透和置顶行为需要在窗口化、无边框和全屏场景分别验证。
