# Vision pipeline design

## Goal

视觉流水线在游戏进程外读取 SC2 窗口画面，把像素转换为可测试、可过期、可归属到
当前对局的语义证据。它不注入游戏进程、不读取进程内存、不模拟输入，也不让截图、
窗口句柄或图像阈值进入 `sc2-copilot-core`。

首个纵向切片实现小地图红色 ping 识别核心和地图分支证据接入。实时 Windows 采集、
真实 ROI 标定和自动启用规则在后续切片完成。

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
2. 仅在 6119 表示对局中、当前地图需要视觉证据且 SC2 可见时采集。
3. 通过 `GetClientRect` 和 `ClientToScreen` 裁出客户区。
4. 排除 SC2 Copilot 覆盖层，避免自身 UI 污染 ROI。
5. 使用容量为一的最新帧槽；消费者落后时丢弃旧帧。
6. 不默认保存完整截图。显式诊断导出也只允许保存裁剪、脱敏后的 ROI。

采集失败必须产生 `UnavailableReason`，不能伪装成“没有检测到”。

### Coordinate model

坐标 module 输入客户区矩形、DPI、像素尺寸和可选黑边，输出统一的内容矩形与锚定
ROI。ROI 使用内容边缘锚点，而不是简单按整屏比例缩放：

```rust
RoiSpec {
    anchor: BottomLeft,
    offset_at_1080p: [i32; 2],
    size_at_1080p: [u32; 2],
}
```

第一批实时支持范围必须由夹具验证后声明；未验证的宽高比、UI 缩放或语言返回
`UnsupportedLayout`。不会静默套用最接近的坐标。

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

真实 ROI、时间窗和位置区域尚未由本项目夹具标定前，不启用实时自动映射。控制器仍
实现并测试稳定视觉分支证据的优先级：

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
- 工作线程最多保留一帧和一份结果。
- 单次小地图 ROI 检测目标 p95 小于 10 ms。
- 整条活跃视觉流水线目标 p95 小于 50 ms。
- 连续采集错误采用有界退避，并发布诊断状态。

## Assets and calibration

所有录制 ROI、模板和标定文件必须：

- 由本项目独立采集；
- 记录分辨率、DPI、UI 缩放、语言、窗口模式、采集日期和许可；
- 正样本和负样本分开；
- 不包含玩家名称、聊天内容或完整游戏截图；
- 开发夹具不进入发布 ZIP，运行时必要的小资源使用 `include_bytes!`。

## Delivery plan

### Phase 1 — recognizer and semantic seam

- 新建 `sc2-copilot-vision`。
- 实现红色分割、连通域、几何筛选和跨帧确认。
- 使用独立生成的 ROI 测试正样本、静态红色对象、单帧噪声、不可用和会话切换。
- 新增 `AppController::handle_vision`，验证会话、目录和手动优先级。

Phase 1 不读取实时画面，也不自动映射真实地图位置。

### Phase 2 — capture and coordinates

- 实现 DXGI 最新帧 adapter。
- 实现客户区、DPI、黑边和锚定 ROI 坐标模型。
- 覆盖窗口化、无边框、全屏、多显示器和 Alt+Tab 夹具/冒烟测试。

### Phase 3 — calibration and live map rules

- 采集两张目标地图的独立正负 ROI。
- 标定颜色、几何、时间窗和位置区域。
- 接通 ping 位置到 `layout-a` / `layout-b` 的映射。
- 在设置和诊断页显示视觉可用性、候选和最终来源。

### Phase 4 — hardening

- 长时间 CPU/内存基准。
- 断网、无 SC2、采集权限失败和显示模式切换。
- 干净 Windows 11 发布包测试。
- 根据误报/漏报数据决定是否保留纯 Rust 实现或重新评估 OpenCV 对照。

## Phase 1 acceptance criteria

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
- 新增视觉与控制器接入代码通过严格 Clippy，现有全量测试保持通过。
