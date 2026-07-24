# Vision pipeline design

## Goal

视觉流水线在游戏进程外读取 SC2 窗口画面，把像素转换为可测试、可过期、可归属到
当前对局的语义证据。它不注入游戏进程、不读取进程内存、不模拟输入，也不让截图、
窗口句柄或图像阈值进入 `sc2-copilot-core`。

当前纵向切片已接通小地图红色 ping 的 Windows 实时采集、跨帧识别、地图规则和
控制器分支证据。

## Architecture

```text
SC2 6119 observation
        |
        v
VisionContext { session, map, game time, enabled recognizers }
        |
        v
Windows capture adapter -- latest frame only
        |
        v
Coordinate model -- SC2 client area -> normalized ROI
        |
        v
sc2-copilot-vision
  MinimapPingRecognizer::observe(frame)
        |
        v
VisionUpdate { session, evidence }
        |
        v
AppController::handle_vision
        |
        v
CopilotEngine / overlay / diagnostics
```

### Capture adapter

Windows 11 上优先使用 DXGI Desktop Duplication；若目标窗口模式或显卡驱动无法稳定
提供客户区，再评估 Windows Graphics Capture。adapter 负责：

1. 查找 `SC2_x64.exe` 的顶层窗口和所在显示器。
2. 仅在 6119 表示对局中、当前地图处于检测时间窗且 SC2 是前台窗口时采集。
3. 通过 `GetClientRect` 和 `ClientToScreen` 直接定位并只读回小地图 ROI。
4. 对本进程所有可见顶层窗口设置 `WDA_EXCLUDEFROMCAPTURE`，避免自身 UI 污染 ROI；
   排除尚未成功时暂停识别，不允许用可能包含覆盖层的画面作“无红点”判定。
5. 使用容量为一的最新帧槽；消费者落后时丢弃旧帧。
6. 不默认保存完整截图。显式诊断导出也只允许保存裁剪、脱敏后的 ROI。

采集支持 8 位 BGRA/RGBA、10 位 UNORM 和 HDR `R16G16B16A16_FLOAT`；优先请求
8 位 BGRA，驱动不支持 `DuplicateOutput1` 时回退到兼容的 `DuplicateOutput` 并在
CPU 端转换。显示模式或 Desktop Duplication 会话失效时丢弃 backend 并在下一次采样
重建。

采集失败必须产生 `UnavailableReason`，不能伪装成“没有检测到”。规范化 ROI 还要通过
最小内容校验；纯黑、近乎均匀或尺寸错误的画面返回 `InvalidMinimap`，因此菜单、转场和
异常读回不会累计有效帧。

### Coordinate model

当前坐标 module 只接受不小于 1280×720 的严格 16:9 客户区，按 1920×1080 基准
`(27, 807, 264, 259)` 缩放并归一化回 264×259。1080p、1440p 和 4K 均有确定性测试；
4K HDR 已在真实 SC2 窗口完成采集冒烟。

非 16:9、低于 720p、旋转显示器会明确返回不支持。黑边与非 100% SC2 UI 缩放尚未
标定，因此当前发布要求游戏使用无黑边的 16:9 客户区和 100% UI 缩放；内容校验只负责
拦截空画面，不能替代这些布局约束。

### Vision module

`sc2-copilot-vision` 是深 module。调用方只需要学习：

```rust
MinimapPingRecognizer::observe(PingFrame) -> PingObservation
```

接口不暴露 HSV 阈值、形态学 kernel、连通域、几何分数或跟踪状态。可用帧包含
`session_id`、严格递增但允许跳号的 `frame_id` 和规范化 ROI；不可用帧包含明确原因。

输出状态：

- `Unavailable`：这一帧无法检测，并给出原因。
- `NoEvidence`：帧有效，但没有满足条件的候选。
- `Candidate`：单帧候选，尚不足以改变业务状态。
- `Confirmed`：跨帧稳定、带规范化位置和置信度的 ping。

切换会话、帧号未向前推进、不可用或无证据都会清空未确认候选。

## Minimap ping algorithm

第一阶段使用传统视觉：

1. RGB 转 HSV，并结合 RGB 通道优势筛选高饱和红色。
2. 对二值掩码做小 kernel 闭运算，连接 ping 外圈的短缺口。
3. 使用 8 邻域连通域标记。
4. 以连通块标签分别评分，要求中央红色 core 与外围菱形边缘分离存在，并用面积、
   包围盒、长宽比、菱形边缘分数和 L1 边缘离散度筛选外圈。
5. 把候选中心转换到 `[0, 1] × [0, 1]` 规范坐标。
6. 按中心距离关联相邻帧；要求连续出现且外圈尺寸/面积发生动画变化才确认。

“相同位置的静态红色单位”不会仅因持续存在而确认；单帧特效和噪声不会越过候选
状态。阈值必须由本项目自建正负 ROI 重新标定，不能复制其他项目的参数。

若同帧有多个合格候选，第一阶段返回几何分最高者；真实夹具出现无法稳定区分的多
候选场景时，结果应改为 `NoEvidence` 或显式歧义，而不是任意选择。

## Map variant policy

识别 module 只输出 ping 位置，不知道地图 ID 或分支 ID。应用层的地图策略在限定游戏
时间窗内，把稳定位置映射为目录中的 `layout-a` / `layout-b`。

首批目标地图：

- `temple-of-the-past`
- `void-rifts`

实时规则：

- `temple-of-the-past`：03:15–03:20 检查小地图左下区域；确认红点选择
  `layout-b`，时间窗内至少取得一个有效检测帧且未确认红点则选择 `layout-a`。
- `void-rifts`：03:00–03:10 检查小地图右侧目标区域；确认红点选择
  `layout-a`，时间窗内至少取得一个有效检测帧且未确认红点则选择 `layout-b`。

程序在时间窗之后首次看到对局时不会猜测分支；整个窗口都无法截图时也不会把
“不可用”解释为“无红点”。控制器按以下优先级处理稳定视觉分支证据：

```text
Manual > Vision > Default
```

规则：

- 证据的 `session_id` 和 `map_id` 必须匹配当前 6119 观察。
- `variant_id` 必须存在于当前地图目录。
- 用户手动选择后，视觉结果不得覆盖。
- 新会话、当前地图改变或返回菜单会清除视觉和手动来源状态。
- 短暂断开 6119 但随后恢复为同一会话时保留手动来源；这与核心引擎的重连语义一致。
- `Unavailable`、`NoEvidence` 和 `Candidate` 不改变分支。

## Runtime and performance

视觉 runtime 只使用一个工作线程，并复用像素缓冲：

- 基础采样上限 10 Hz。
- 红点识别只在目标地图的短时间窗启用。
- 工作线程不建立帧队列，只保留最新上下文、状态和一份尚未消费的结果。
- 单次小地图 ROI 检测目标 p95 小于 10 ms。
- 整条活跃视觉流水线目标 p95 小于 50 ms。
- 采集错误会重置 DXGI backend、清除未确认候选并发布诊断状态；下一次 10 Hz 采样重试。

## Assets and calibration

所有录制 ROI、模板和标定文件必须：

- 由本项目独立采集；
- 记录复现算法所需的分辨率、DPI/UI 缩放、客户端语言、窗口模式、像素格式、采集日期和素材许可；
- 正样本和负样本分开；
- 不包含玩家名称、账号标识、聊天内容、完整端点响应或完整游戏截图；
- 不记录本机绝对路径、设备序列号、显示器名称或其他可关联到具体操作者的元数据；
- 只有显式标定操作可以保存裁剪 ROI，分享前必须再次人工复核；
- 开发夹具不进入发布 ZIP，运行时必要的小资源使用 `include_bytes!`。

## Delivery plan

### Phase 1 — recognizer and semantic seam

- 新建 `sc2-copilot-vision`。
- 实现红色分割、连通域、几何筛选和跨帧确认。
- 使用独立生成的 ROI 测试正样本、静态红色对象、单帧噪声、不可用和会话切换。
- 新增 `AppController::handle_vision`，验证会话、目录和手动优先级。

Phase 1 已完成。

### Phase 2 — capture and coordinates

- 实现 DXGI 最新帧 adapter。
- 实现 16:9 客户区到规范小地图 ROI 的坐标模型。
- 覆盖 1080p、1440p、4K、HDR、前台切换和最小化行为。

Phase 2 的 DXGI adapter、16:9 坐标模型、前台/最小化门控、HDR 转换和覆盖层捕获排除
已完成；非 16:9、旋转显示器和更多窗口模式仍属于兼容性加固。

### Phase 3 — calibration and live map rules

- 用独立生成的正负 ROI 标定颜色和几何规则。
- 实现两张目标地图的时间窗和位置区域。
- 接通 ping 位置到 `layout-a` / `layout-b` 的映射。
- 在设置和诊断页显示视觉可用性、候选和最终来源。

Phase 3 的两张地图规则、时间窗、有效帧缺席语义、控制器接入和诊断状态已完成。
虚空撕裂的真实无红点路径已完成端到端验证；实战标定记录见
[minimap-red-ping.md](calibration/minimap-red-ping.md)。真实目标区域红点和往日神庙
仍是后续标定项；长期误报/漏报数据在 Phase 4 持续积累。

### Phase 4 — hardening

- 长时间 CPU/内存基准。
- 断网、无 SC2、采集权限失败和显示模式切换。
- 干净 Windows 11 发布包测试。
- 根据误报/漏报数据决定是否保留纯 Rust 实现或重新评估 OpenCV 对照。

## Current acceptance criteria

- 一个有效动画 ping 序列进入并保持 `Confirmed`。
- 单帧 ping 只产生 `Candidate`。
- 相同位置的静态红色对象不会确认。
- 红色散点、实心矩形和扩张圆环不会成为候选。
- 只有外圈而没有中央红色 core 的对象不会成为候选。
- 最新帧槽丢帧导致的 `frame_id` 跳号不会中断有效动画。
- 空 ROI 返回 `UnsupportedLayout`；截图不可用与无证据可区分。
- 新会话不会继承旧候选。
- 控制器拒绝旧会话、错误地图和未知分支证据。
- 手动分支不会被视觉证据覆盖；同会话短暂重连时仍保留，换图或新会话时清除。
- 只有两张目标地图的检测时间窗会启动 DXGI 采集。
- 时间窗内没有任何有效帧时不发布“无红点”分支；晚于时间窗加入时不猜测。
- 1080p、1440p 和 4K 小地图坐标通过测试，4K HDR SC2 真实对局 ROI 已通过内容校验。
- 覆盖层、设置和锁定按钮窗口被排除在 Desktop Duplication 捕获之外。
- 新增视觉与控制器接入代码通过严格 Clippy，现有全量测试保持通过。
