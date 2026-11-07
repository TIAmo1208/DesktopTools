# UX/UI 工具调用

需要产出或修改界面设计（Penpot 画板、线框、设计稿评审）时读本页。本页只管**怎么调用工具、怎么避免返工**；设计流程见 `docs/design/设计确认流程.md`，视觉数值见 `docs/design/视觉规范.md`，落地实现见 `penpot-design-to-code` 技能。

## 工具分工

| 工具 | 用来做什么 | 不要用来做什么 |
| --- | --- | --- |
| Penpot MCP `execute_code` | 读写画板、节点、样式；批量建图、批量布局、自检 | 用 `snapshot` 读设计内容 |
| Penpot MCP `export_shape` | 导出画板 PNG 做视觉核对 | 代替结构数据判断尺寸与坐标 |
| ego-browser | 唤醒或操作 Penpot 标签页、点开 MCP 插件 | 用截图量测设计坐标 |

## 开工前：三件事一次做完

1. **连接 Penpot**：`execute_code` 报 `No Penpot instance connected` 时，用 ego-browser 打开 `https://design.penpot.app/`，点左侧工具栏的 `MCP` 按钮，页面出现「MCP 已连接」再继续。不要反复调用 MCP 工具试连接。
2. **锁定目标画板**：`penpot.currentPage` 不一定是你以为的那一页。先 `penpotUtils.getPages()` 与 `penpot.currentPage.root.children.filter(c => c.type === 'board')` 列出画板名与坐标，确认改哪个再动手。
3. **读尺寸再布局**：所有画板一次取全部尺寸，`x` 按「前一个右边界 + 间距」累加算出计划，最后一次性写入。不要「改一张—看一眼」地逐张挪位。

## 坐标语义（最容易踩的坑）

跨调用写坐标时，**统一用 `penpotUtils.setParentXY(shape, x, y)` 写父级相对坐标**。

- `shape.x` / `shape.y` 在 `appendChild` 之后按绝对坐标生效，行为随上下文变化；混用绝对与相对会让整批文字偏出一个画板宽度。
- 一次 `execute_code` 里先建对象再统一赋值，比逐条 `createX` 时带坐标更稳。
- 创建文本用 `penpot.createText("内容")`（带参），创建后再设 `fontSize`、`fills`、`growType`。
- 字体用 `penpot.fonts.findByName("Source Sans Pro")` + `font.applyToText(text, variant)`，不要猜 `fontFamily` 字符串。

## 写画板的纪律

- **先算后写**：一屏内容先算出每格的 `x / y / w / h` 与文字基线，再一次性建完。中途加元素时按格重算，不做「整体平移 N 像素」的补丁。
- **禁止批量平移**：用 `forEach(s => s.x += N)` 修正布局会误伤已正确的元素。发现偏差就按元素定位（按文字内容或外框）精确赋值。
- **文字不压图形**：每条说明文字放在对应格子外框之外的下方，与外框留 8–12px；同一行多条说明用等差 y 排开。
- **不越界**：最后一个元素右边界 ≤ 画板宽度 − 32，底部同理；画板高度按内容末尾 + 32 收口。

## 每张画板写完就跑自检

一次性跑完，不要靠肉眼看图判断有没有遮挡：

```js
const boards = penpot.currentPage.root.children.filter(c => c.type === 'board');
const rs = boards.map(b => ({ n: b.name, x: b.x, y: b.y, r: b.x + b.width, b: b.y + b.height }));
const overlaps = [];
for (let i = 0; i < rs.length; i += 1)
    for (let j = i + 1; j < rs.length; j += 1) {
        const a = rs[i], b = rs[j];
        if (a.x < b.r && b.x < a.r && a.y < b.b && b.y < a.b) overlaps.push([a.n, b.n]);
    }
const oob = boards.map(b => ({
    n: b.name,
    count: b.children.filter(s => s.x < b.x - 2 || s.y < b.y - 2 || s.x + s.width > b.x + b.width + 2 || s.y + s.height > b.y + b.height + 2).length,
})).filter(x => x.count > 0);
return { overlaps, oob };   // 两项都空才算这轮完成
```

自检通过后再 `export_shape` 看一次图，确认视觉层级与文字重叠。导出前对目标画板设 `penpot.selection = [board]`，`shapeId` 传 `selection`。

## 一轮完成的检查表

- [ ] Penpot 已连接，目标页与画板已确认
- [ ] 所有画板尺寸已取，位置已算好并一次性写入
- [ ] 文字与外形无重叠，无越界
- [ ] 自检返回 `overlaps: []`、`oob: []`
- [ ] 每张画板导出过一次并对图核对
- [ ] 设计已落到 `docs/design/` 对应文档，并与 `CONTEXT.md` 条目一致

## 历史踩坑

| 现象 | 原因 | 做法 |
| --- | --- | --- |
| 整批文字偏向画板右侧 | 写了父级相对坐标当绝对坐标 | 统一 `setParentXY` |
| 修一处、坏三处 | 用整体平移补丁修元素 | 按文字内容或外框精确重定位 |
| 说明文字压住柱子与标题 | 文字 y 与外框内容区重叠 | 说明放外框外下方，行间 22px |
| 画板底部被裁 | 画板高度未随内容增长 | 建完收口时 `board.resize(W, 内容末尾 + 32)` |
| 改完发现改错页 | 默认信任 `currentPage` | 先列画板清单确认 |
