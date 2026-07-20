# Keiframe 地图时间表清单与 clean-room 比较边界

## 结论摘要

- 固定基线 `192bdbce6868e597b297cf47f485ac5c79eb9baf` 的 `maps.db` 包含 **18 个配置、345 条记录**；去掉三个项目自定义分支后，对应 **15 张基础地图**。
- 17 个配置按“游戏开始后的经过秒数”工作；`净网行动` 是例外，它按“已净化节点数 + 当前阶段剩余倒计时”工作，并依赖 OCR 提供这两个状态。它不能与普通地图共用同一种时间语义。
- 单条记录本质上是一个扁平、面向展示的提醒行，不是强类型游戏事件：`event_text` 与 `army_text` 混合了事实、方向、科技等级、生命值、混合体、备注和 Keiframe 自定义简写。
- clean-room 流程可以用 Keiframe 检查灰机 Wiki 是否漏了时间点、同时事件、地图分支或候选属性；不能复制数据库行、提示文案、简写、声音、风暴英雄编码、识别参数或实现算法。发生冲突时仍采用已决定的“灰机 Wiki 基线，Keiframe 只报差异”策略。

## 调查边界与可复现基线

本报告只检查本地仓库 `reference-checkout` 的固定提交：

```text
commit: 192bdbce6868e597b297cf47f485ac5c79eb9baf
commit time: 2026-06-24T00:34:21+08:00
maps.db git blob: 4c0aa31953c75551320557f0568aeef02508e5a3
maps.db SHA-256: FB8BF7066013EA233356C46C8E844476E2F443B587BFCFDC5F0C7410EE448975
```

主要一手来源是固定提交中的 [`resources/db/maps.db`](reference-checkout/resources/db/maps.db)、数据库读取代码 [`map_daos.py`](reference-checkout/src/db/map_daos.py) 和实际消费代码 [`map_loader.py`](reference-checkout/src/map_handlers/map_loader.py)、[`map_event_manager.py`](reference-checkout/src/map_handlers/map_event_manager.py)、[`malwarfare_event_manager.py`](reference-checkout/src/map_handlers/malwarfare_event_manager.py)。未查询或引用灰机 Wiki。

数据库统计可用以下只读查询复现：

```sql
SELECT COUNT(*) AS records, COUNT(DISTINCT map_name) AS configs
FROM map_configs;

SELECT map_name,
       COUNT(*) AS records,
       SUM(COALESCE(sound_filename, '') <> '') AS sound_rows,
       SUM(COALESCE(hero_text, '') <> '') AS hero_rows,
       MIN(time_value) AS min_time,
       MAX(time_value) AS max_time
FROM map_configs
GROUP BY map_name
ORDER BY map_name;
```

## 18 个配置到 15 张基础地图的映射

记录数、声音行数和风暴英雄行数来自上述固定 `maps.db` 查询。`min..max` 是数据库 `time_value` 的秒数范围；对 `净网行动`，它表示阶段内剩余倒计时阈值，不是从开局起经过的秒数。

| Keiframe 配置 | 基础地图 | 记录 | `time_value` 范围 | 声音行 | 风暴英雄行 | 主要形状 |
|---|---|---:|---:|---:|---:|---|
| 亡者之夜 | 亡者之夜 | 7 | 840..3555 | 0 | 3 | 长局防守波次；方向与 `T` 等级 |
| 克哈裂痕 | 克哈裂痕 | 11 | 120..1800 | 1 | 9 | 进攻波次和分矿目标混排 |
| 净网行动 | 净网行动 | 31 | 1..150 | 0 | 1 | `count=0..4` 的阶段内剩余倒计时；波次、压制塔和说明行混排 |
| 升格之链 | 升格之链 | 29 | 210..2340 | 1 | 20 | 护卫、红点、分叉、混合体组合 |
| 天界封锁 | 天界封锁 | 17 | 240..2460 | 0 | 8 | 左/右方向和 `T` 等级 |
| 往日神庙-A | 往日神庙 | 28 | 195..1480 | 0 | 9 | A 分支；普通波次、空投、撕裂和多方向同时事件 |
| 往日神庙-B | 往日神庙 | 28 | 190..1480 | 0 | 0 | B 分支；时间和方向与 A 分支部分不同 |
| 机会渺茫-人虫 | 机会渺茫 | 32 | 180..1725 | 0 | 0 | 人族/虫族敌军节奏；多位置候选与红点事件 |
| 机会渺茫-神 | 机会渺茫 | 32 | 180..1705 | 3 | 0 | 神族敌军节奏；与人虫分支有秒级差异 |
| 死亡摇篮 | 死亡摇篮 | 11 | 60..1980 | 0 | 0 | 一条方向说明，加固定 `T` 波次 |
| 湮灭快车 | 湮灭快车 | 19 | 240..1500 | 0 | 9 | 进攻波次、主/奖励列车；生命值和混合体被压入简写 |
| 熔火危机 | 熔火危机 | 15 | 60..2520 | 1 | 6 | 一条时间波动说明，加左右侧波次 |
| 聚铁成兵 | 聚铁成兵 | 10 | 60..1800 | 0 | 9 | 一条条件说明，加路径、目标和波次 |
| 营救矿工 | 营救矿工 | 17 | 225..1920 | 0 | 1 | 飞船发射、敌军冲船与普通波次混排 |
| 虚空撕裂-右 | 虚空撕裂 | 17 | 240..2460 | 0 | 8 | 右分支；四次撕裂与普通波次 |
| 虚空撕裂-左 | 虚空撕裂 | 18 | 180..2520 | 0 | 8 | 左分支；18:00 有两个同时事件 |
| 虚空降临 | 虚空降临 | 13 | 180..1458 | 1 | 10 | 普通波次、奖励目标和混合体数量 |
| 黑暗杀星 | 黑暗杀星 | 10 | 60..1560 | 0 | 1 | 一条敌方种族提示，加方向、波次和混合体数量 |

三组拆分规则在实现中也被明确当作版本按钮：`左/右`、`A/B`、`神/人虫`。参见 [`map_loader.py:130`](reference-checkout/src/map_handlers/map_loader.py:130)。其中：

- `往日神庙-A/B` 和 `虚空撕裂-左/右` 还存在小地图红点自动判定规则，固定时间窗口、ROI、阈值、目标分支和提示文案均写在 [`map_variant_auto_resolver.py:18`](reference-checkout/src/map_handlers/map_variant_auto_resolver.py:18) 与 [`map_variant_auto_resolver.py:51`](reference-checkout/src/map_handlers/map_variant_auto_resolver.py:51)。这些属于视觉识别实现，不属于时间表事实。
- `机会渺茫-人虫/神` 根据敌方种族状态自动切换，参见 [`map_loader.py:81`](reference-checkout/src/map_handlers/map_loader.py:81)。两份配置各 32 行，但很多事件相差数秒，不能只保留一份时间表再改名称。

## 数据库记录形状

固定数据库的实际结构是：

```sql
CREATE TABLE maps (
    map_name TEXT PRIMARY KEY
);

CREATE TABLE map_configs (
    map_name       TEXT NOT NULL,
    time_label     TEXT NOT NULL,
    time_value     INTEGER NOT NULL,
    count_value    INTEGER,
    event_text     TEXT,
    army_text      TEXT,
    sound_filename TEXT,
    hero_text      TEXT,
    PRIMARY KEY (map_name, time_value, event_text),
    FOREIGN KEY (map_name) REFERENCES maps(map_name)
        ON DELETE CASCADE ON UPDATE CASCADE
);
```

主键包含 `event_text`，所以同一地图同一秒可以有多条记录。固定快照中共有 **10 组**同秒多事件：9 组属于 `净网行动`，另一组是 `虚空撕裂-左` 的 18:00（普通波次与第 4 次撕裂）。不能将 `(map, second)` 当作唯一事件键。

DAO 将每行原样改造成以下展示模型，没有解析事件类型。参见 [`map_daos.py:33`](reference-checkout/src/db/map_daos.py:33)：

```text
map_name
time { label, value }
count
event
army
sound
hero
```

普通地图按 `time_value ASC` 排序；`净网行动` 按 `count_value ASC, time_value DESC` 排序，并用 `event_text` 是否以 `T` 开头参与同阶段排序。参见 [`map_daos.py:4`](reference-checkout/src/db/map_daos.py:4)。这进一步说明 `event_text` 既是展示文本，又被 Keiframe 当作隐含类型标签，数据库本身没有类型字段。

## 字段的实际用途与性质

| 字段 | Keiframe 中的实际用途 | 性质判断 | clean-room 处理 |
|---|---|---|---|
| `map_name` | 选择配置、列出地图、构造分支按钮 | 基础地图名是领域事实；`-A/-B/-左/-右/-神/-人虫` 是 Keiframe 的分支标识约定 | 可以比较基础地图与分支覆盖；自行设计稳定 ID，不复制其命名约定作为内部协议 |
| `time_label` | 显示在表格中；事件管理器重新解析它并据此触发提醒 | 时间文本同时是运行输入和展示值 | 只比较规范化后的秒数；不要保留原有补零风格。固定快照 345 行的 label/value 均一致，但数据库没有一致性约束 |
| `time_value` | 普通地图排序；`净网行动` 的倒序排序；导入时由 label 计算 | 普通地图是开局经过秒；净网是阶段剩余倒计时阈值 | 分开建模两种时钟语义，不能放进一个无条件 `timestamp` 字段 |
| `count_value` | 仅 `净网行动` 作为“已净化节点数”展示和匹配条件 | 净网专用的事实条件 | 只在净网的阶段条件中比较；不要把空字符串当合法阶段 |
| `event_text` | 表格事件列、Toast 文案、主键；净网排序还检查 `T` 前缀 | 事实与展示混合，包含方向、事件、说明、软件局限和简写 | 不能逐字迁移；只作为人工差异提示，事件分类以 Wiki 表结构重新建立 |
| `army_text` | 表格补充列并拼入 Toast | 展示简写，混合科技等级、生命值、混合体数量、空陆信息等 | 可拆出候选维度做差异检查；值仍以 Wiki 为准，不复制组合语法 |
| `sound_filename` | 普通地图即将发生时传给 Toast/声音播放 | Keiframe 展示资产引用 | 不进入地图事实快照，不复制文件名或音频资源 |
| `hero_text` | 仅在 `HeroesFromtheStorm` 激活时追加到普通地图 Toast | 突变因子特有、Keiframe 自定义编码 | 从本次基础地图时间表排除，未来若实现该突变需独立研究和建模 |

字段的人类可读标题也明确把 `count_value` 标为“净网专用”、把 `army_text` 标为“补充（科技等级/混合体/...）”、把 `hero_text` 标为“风暴英雄”，参见 [`temp_translate_utils.py:2`](reference-checkout/src/utils/temp_translate_utils.py:2)。

### `time_label` 与 `time_value` 的真实关系

导入代码把 `MM:SS` 转成秒并同时写入两列，参见 [`map_daos.py:90`](reference-checkout/src/db/map_daos.py:90)。但运行时普通事件管理器读取表格中的 `time_label` 并再次解析，不直接使用 DAO 返回的 `time_value`；提醒条件是 `event_second - current_game_second`，参见 [`map_event_manager.py:43`](reference-checkout/src/map_handlers/map_event_manager.py:43) 与 [`map_event_manager.py:106`](reference-checkout/src/map_handlers/map_event_manager.py:106)。因此：

- 两列不是两个独立事实；应归一成一个整数秒字段，并在展示时格式化。
- 数据库当前虽然没有差异，但仅复制 `time_value` 而忽略 label 的运行语义，可能无法准确解释旧实现。

### 普通地图事件

普通地图的每一行使用开局经过秒：程序寻找下一个时间点、把过去行置灰，并在事件前的配置窗口内显示 `time + event + army`；`sound` 被作为可选提醒音，`hero` 仅在 `HeroesFromtheStorm` 激活时追加。参见 [`map_event_manager.py:42`](reference-checkout/src/map_handlers/map_event_manager.py:42)、[`map_event_manager.py:65`](reference-checkout/src/map_handlers/map_event_manager.py:65) 和 [`map_event_manager.py:106`](reference-checkout/src/map_handlers/map_event_manager.py:106)。

这类行至少混合以下形状，数据库没有显式枚举：

- 普通进攻波次：方向/来向 + `T1..T7`；
- 地图目标：列车、飞船、撕裂、奖励、压制塔等；
- 复合波次：普通部队 + 小/大混合体、空军、生命值；
- 多事件或多方向：`+`、`/`、`&`、箭头等符号拼接；
- 说明行：时间波动、分支判断提示、软件无法提示的情况。

因此，不能用 `event_text` 的原始字符串模式作为新项目的领域模型。

### `净网行动` 的不同时间语义

`净网行动` 由加载器选择专用事件管理器，参见 [`map_loader.py:99`](reference-checkout/src/map_handlers/map_loader.py:99)。专用管理器只处理与当前 `count` 相等的行，并用：

```text
time_diff = current_countdown_seconds - row_threshold_seconds
```

判断即将到达的阶段内倒计时阈值，参见 [`malwarfare_event_manager.py:51`](reference-checkout/src/map_handlers/malwarfare_event_manager.py:51) 与 [`malwarfare_event_manager.py:118`](reference-checkout/src/map_handlers/malwarfare_event_manager.py:118)。`current_count` 和倒计时来自 OCR 数据，而不是普通游戏经过时间，参见 [`game_time_handler.py:44`](reference-checkout/src/game_time_handler.py:44) 与 [`game_time_handler.py:67`](reference-checkout/src/game_time_handler.py:67)。

固定数据库中的有效 `count_value` 分布为：`0` 两行、`1` 六行、`2` 九行、`3` 七行、`4` 七行。`count=0` 包含识别提示和软件局限说明，不全是可触发的游戏事件。

初版已排除视觉识别，因此即使抓到了 Wiki 表格，也不能声称已具备 Keiframe 同等的净网自动提醒条件。可先保存原始表格并把该事件模型列为待设计；不要把 1..150 秒误解释成开局前 2 分 30 秒的绝对事件。

## 地图特有差异与建模提示

### 三组分支配置

1. **往日神庙 A/B**：两份均为 28 行，但第一波分别为 3:15 与 3:10，后续多处方向、时间和事件组合不同；A 有 9 行 `hero_text`，B 没有。这是两份不同时间表，不是单一表的显示别名。
2. **机会渺茫 人虫/神**：两份均为 32 行，事件序列相似但大量时间有数秒差；神分支另有 3 行声音。分支条件来自敌方种族，不能用“随机分支”泛化。
3. **虚空撕裂 左/右**：左 18 行、右 17 行；左分支 18:00 同时出现普通波次和第 4 次撕裂。比较器必须保留同秒多事件，并以事件内容或来源行标识，而不是只按秒匹配。

### 特殊或混合记录

- `亡者之夜` 的时间可到 59:15，普通时间字段不能假设少于一小时。
- `湮灭快车` 的 `army_text` 将等级、近似生命值和混合体压进同一个字符串，例如 `T`、`k`、`黑/红` 组合。这些是差异线索，不是可复用格式。
- `营救矿工` 把“飞船发射”“敌军正冲向飞船”和普通 `T` 波次放在同一列；仅凭时间和文本无法稳定推断事件类型。
- `熔火危机`、`死亡摇篮`、`聚铁成兵`、`黑暗杀星` 都包含说明行。比较时必须区分“定时事件”和“用户提示/限制说明”。
- `升格之链` 与 `往日神庙` 的一行可能同时描述多个方向、部队等级、混合体或空投；事件不是一行对应一个原子事实。
- `聚铁成兵` 与 `虚空撕裂-左` 的 `count_value` 实际存储为空字符串，尽管列声明为 `INTEGER`；加载器对普通地图也不消费该列。这是旧数据形状，不应当作跨地图通用字段。
- `sound_filename` 在 7 行为 `Default.mp3`；`hero_text` 则既有名字简写，也有数字串、`无法判断`、`剩余全部`、`本图无风暴` 等控制/展示语义。两者均不能作为基础时间表事实。

## clean-room 差异检查规则

### 可以安全比较的维度

下列维度只用于比较两个来源，最终发布值仍来自灰机 Wiki：

1. **覆盖维度**：15 张基础地图是否齐全；Wiki 页面是否表达三组分支。
2. **时间维度**：将时间文本规范化为整数秒后，比较缺失、额外和偏移；`净网行动` 必须加阶段条件并使用倒计时语义。
3. **多事件维度**：同一秒的事件数量、所属表格和原始 Wiki 行身份，防止被字典键覆盖。
4. **事件类别维度**：进攻波次、主目标、奖励目标、护卫/防守、说明行；分类首先来自 Wiki 表头和单元格，Keiframe 仅给出候选差异。
5. **候选属性维度**：方向/轨道、目标、科技或规模等级、精确生命值、混合体数量。Keiframe 的组合字符串只能用于提示人工核查，不能直接成为发布值。
6. **分支维度**：分支数量、分支条件和各分支的时间序列是否不同；不比较或复用视觉判定算法。

建议比较器输出至少区分：`match`、`wiki_only`、`keiframe_only`、`time_mismatch`、`attribute_mismatch`、`ambiguous_keiframe_shorthand`、`unsupported_runtime_condition`。其中：

- `keiframe_only` 只进入差异报告，不自动补进规范化快照；
- `ambiguous_keiframe_shorthand` 不尝试自动“解码”为事实；
- `unsupported_runtime_condition` 用于 `净网行动` OCR 条件和已排除的视觉分支条件。

### 不可迁移内容

- 任意 Python 源码、控制流、类/函数接口、SQL 表结构、主键设计或 DAO 展示模型；
- `maps.db` 二进制文件、整行导出、原始记录顺序；
- `event_text`、`army_text`、`hero_text` 的原文、简写体系、符号组合和提示语；
- `sound_filename` 及对应音频资产；
- `map_keywords` 中的项目别名，例如“火车”“庙a”“撕裂b”；
- 小地图 ROI、监测时间窗、阈值、红点判定、自动分支切换与提示文案；
- 由敌方种族识别触发的自动配置切换实现；
- `HeroesFromtheStorm` 专用编码以及其他非基础地图时间表能力。

## 对后续全地图抓取设计的约束

在看到全部 Wiki 表格前，不宜确定单一强类型载荷枚举；本次 Keiframe 清单已经证明至少要先解决四个独立问题：

1. 普通开局经过时间与阶段剩余倒计时是两种时钟模型；
2. 同秒可以有多个事件，一行也可能含多个事实；
3. 基础地图、地图分支和分支判定条件应分开；
4. 事实、来源原文、展示文案和功能特有附加信息不能混在同一字符串中。

抓取完 Wiki 的全部表格后，应以 Wiki 的实际表头、合并单元格和分支表达确定规范化结构；本报告只作为 Keiframe 差异检查字段清单，不应反向决定新项目的数据模型。
