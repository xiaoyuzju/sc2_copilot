# Update Map Schedule Data

本手册用于重复执行“灰机 Wiki 基线抓取 → Keiframe 差异检查 → 发布版本化快照”。它是开发和发布流程，不会由 SC2 Copilot 在玩家机器上运行。

## 当前 Rust 工具流程

完成浏览器 DOM 抓取后，在仓库根目录按顺序执行：

```powershell
$keiframeRoot = (Resolve-Path -LiteralPath $env:KEIFRAME_ROOT).Path
cargo run -p schedule-data -- validate-snapshots data/sources/huiji/2026-07-21
cargo run -p schedule-data -- compile data/sources/huiji/2026-07-21 data/maps/catalog.json data/maps/coverage-report.json
cargo run -p schedule-data -- validate data/maps/catalog.json
git -C $keiframeRoot rev-parse HEAD
cargo run -p schedule-data -- diff-keiframe data/maps/catalog.json $keiframeRoot data/diffs/keiframe-192bdbce-2026-07-21.json
```

把日期替换为新批次日期，但 Keiframe commit 未经重新决策不得替换。`compile` 只读取灰机 Wiki 快照；`diff-keiframe` 只读取已经生成的目录和固定数据库，且只生成差异报告。它没有写回目录的接口。

每次运行应检查：快照验证统计、编译后的地图/相关表/事件数量、`unclassified_row_count=0`、每张表的 `handled_rows` 与 `unsupported_rows` 互斥且并集等于 `source_row_count`、目录来源引用校验，以及差异报告中的固定 commit。生成的 `catalog.json` 是唯一构建输入；`coverage-report.json` 和 `data/diffs` 仅供审查。

## 固定输入

- 灰机 Wiki 总览：<https://starcraft.huijiwiki.com/wiki/%E5%90%88%E4%BD%9C%E4%BB%BB%E5%8A%A1>
- 地图页面格式：`https://starcraft.huijiwiki.com/wiki/合作任务/<地图名>`
- Keiframe 本地参照：由 `KEIFRAME_ROOT` 环境变量指向的只读 checkout
- Keiframe 固定基线：commit `192bdbce6868e597b297cf47f485ac5c79eb9baf`
- Keiframe 时间表数据库：`resources/db/maps.db`

每次更新开始前记录采集日期，设置 `KEIFRAME_ROOT`，并用下面的只读命令确认参照版本。若输出不是固定 commit，停止比较；不要自动切换或修改参照仓库。

```powershell
$keiframeRoot = (Resolve-Path -LiteralPath $env:KEIFRAME_ROOT).Path
git -C $keiframeRoot rev-parse HEAD
```

## 快照产物

实现数据工具时使用以下目录约定：

```text
data/
  sources/huiji/<YYYY-MM-DD>/manifest.json
  sources/huiji/<YYYY-MM-DD>/<map-slug>.json
  maps/<map-slug>.json
  diffs/keiframe-192bdbce-<YYYY-MM-DD>.json
```

- `sources/huiji` 保存网页中实际读取到的原始表格单元格，不进行含义推断。
- `maps` 保存审核后的规范化 JSON，同一时间的多个事件必须保持为多个条目；构建时由 Rust 嵌入可执行文件。
- `diffs` 保存比较结果；它不能修改 `maps` 中采用的灰机 Wiki 值。
- 只有 `maps` 是应用构建输入；`sources` 和 `diffs` 只用于审查，不嵌入应用。
- Git 历史保存旧版本。不要用同名文件覆盖同一天已审核的原始快照；需要重抓时在日期后增加 UTC 时间或序号。

## 第一步：获取地图页面列表

直接 HTTP 请求和 MediaWiki API 在 2026-07-21 的本次调查中返回了 `403`。已验证可行的方法是用正常浏览器打开页面，等待正文渲染完成，再从页面 DOM 读取表格。若网站以后恢复稳定 API，可以新增抓取适配器，但必须先证明其输出与 DOM 快照等价。

在浏览器开发者工具 Console 执行下面的表达式；自动化浏览器应执行同样的页面内 JavaScript。保存返回的地图名和绝对 URL，不手工拼写地图清单。

```javascript
(() => {
  const root = document.querySelector("#mw-content-text");
  const clean = (value) => value.replace(/\s+/g, " ").trim();
  const links = [...root.querySelector("table").querySelectorAll("a[href]")]
    .map((link) => ({
      name: clean(link.textContent),
      url: new URL(link.href, location.href).href,
    }));
  const urls = [...new Set(links.map(({ url }) => url))];

  return urls.map((url) => ({
    name: links.find((item) => item.url === url && item.name)?.name ?? "",
    url,
  }));
})()
```

总览正文还包含大量指挥官、单位和机制链接，不能只按 `/wiki/合作任务/` URL 前缀筛选。当前地图导航是正文中的第一张表；同一地图的图片和文字会形成两个链接，所以上述脚本按 URL 去重并优先保留非空文字。结果应为 15 张基础地图。数量、名称或 URL 集合变化是人工审核信号，不应静默接受。

## 第二步：抓取每张地图的原始表格

逐页打开第一步得到的 URL，等待 `#mw-content-text` 出现，然后执行：

```javascript
(() => {
  const clean = (value) => value.replace(/\s+/g, " ").trim();

  return {
    schema_version: 1,
    index_name: "<从总览清单传入的地图名>",
    source_url: location.href,
    retrieved_at: new Date().toISOString(),
    title: clean(document.querySelector("#firstHeading")?.textContent ?? ""),
    page_last_modified: clean(
      document.querySelector("#footer-info-lastmod")?.textContent ?? ""
    ),
    tables: [...document.querySelectorAll("#mw-content-text table")].map(
      (table, table_index) => ({
        table_index,
        caption: clean(table.caption?.textContent ?? ""),
        classes: [...table.classList],
        rows: [...table.rows].map((row) =>
          [...row.cells].map((cell) => ({
            kind: cell.tagName.toLowerCase(),
            text: clean(cell.textContent),
            row_span: cell.rowSpan,
            column_span: cell.colSpan,
          }))
        ),
      })
    ),
  };
})()
```

实际自动化时应把 `index_name` 作为页面内求值参数传入，不能把示例占位符写进快照。把结果原样保存为该地图的原始 JSON。必须保留 `kind`、`row_span` 和 `column_span`，否则无法可靠区分表头或还原合并单元格；不要只保存屏幕上看起来完整的逐行文本。

2026-07-21 的页面没有提供稳定可读取的 `page_last_modified`，因此本次快照以每页 `retrieved_at` 为采集时间。该字段为空不是抓取失败，不能自行填写修订日期。

## 第三步：规范化应用数据

规范化时遵守以下规则：

1. 每个地图 adapter 按已审核快照显式声明相关表序号和预期时间列；页面表结构变化后必须重新审核，不能自动套用到新序号。
2. 将 `m:ss` 时间转换为秒，同时保留原始时间文本用于审查。
3. 生命值、规模、科技等级和数量按页面精确值保存，不转换成 `k` 简写。
4. 合并单元格只向其覆盖范围展开；相同时间的不同事件不得合并。
5. 保留页面明确给出的条件、分支、轨道和目标；不增加由表格结构猜测出的事件。
6. 使用灰机 Wiki 的方向词汇，不推断它与 Keiframe “上/下”等简写的坐标映射。
7. 只采集时间表事实，不复制页面说明文字、攻略描述或其他表达性内容。
8. 每个地图快照保存 `source_url`、`retrieved_at` 和数据格式版本。

如果某页没有可识别的时间表、字段含义不明确或合并单元格无法确定，应将该页标记为需要人工处理，不生成猜测值。

## 第四步：与 Keiframe 固定基线比较

使用 `sqlite3` 只读导出参照记录。为避免 PowerShell 中中文 SQL 字面量的编码差异，统一导出全部地图后按结构化字段匹配。

```powershell
$keiframeRoot = (Resolve-Path -LiteralPath $env:KEIFRAME_ROOT).Path
$keiframeDb = Join-Path $keiframeRoot 'resources\db\maps.db'
sqlite3 -json $keiframeDb `
  "SELECT map_name, time_label, time_value, event_text, army_text FROM map_configs ORDER BY map_name, time_value, rowid;"
```

Rust 比较器读取 `time_label`、`time_value`、`count_value`、`event_text` 和 `army_text`，但生成的报告不包含后三者的原文。报告从以下维度给出计数、时间点或需人工复核的标记：

- 时间集合、窗口/亚秒精度、`time_label` 与 `time_value` 一致性；
- 同刻记录数量差异和多事实/复合记录数量，用于发现结构压缩；
- 相同时间点的结构化数值候选差异；候选解析不反向写入应用事实；
- 两边词汇 token 的交集和各自独有数量，不输出 Keiframe 词汇表；
- 净网行动的节点数/阶段剩余倒计时被明确标记为首版不支持的运行条件。

冲突时保留灰机 Wiki 值；差异文件只保存归一化诊断与参照 commit，不保存 Keiframe 的 `event_text`、`army_text`、整行记录或词汇表。不要把这些字段直接转换成应用提示文案。

## 第五步：审核与发布

一次更新只有同时满足以下条件才能进入应用数据：

- manifest 中的地图 URL 集合经过人工确认；
- 每张地图都有来源 URL、UTC 采集时间和原始表格快照；
- 合并单元格已展开，且同时时间事件没有被压扁；
- 所有规范化记录都能追溯到原始表格单元格；
- 差异报告引用正确的 Keiframe commit，且没有用参照值覆盖灰机 Wiki；
- 原始快照、规范化数据和差异报告在同一次变更中接受审查；
- Rust 构建能够解析并嵌入全部规范化 JSON，格式校验测试通过；
- 应用从内嵌数据离线启动，既不依赖外部时间表文件，也没有新增网络依赖。

发布记录应写明灰机 Wiki 采集日期和内嵌数据快照版本。若抓取失败，继续构建并使用上一个已审核快照，不在应用启动时临时访问网站。
