# 工单系统：本地 Markdown

需求、规格和工单保存在 `.scratch/` 下的 Markdown 文件中。

## 目录约定

- 每项功能使用独立目录：`.scratch/<feature-slug>/`
- 功能规格：`.scratch/<feature-slug>/spec.md`
- 实施工单：`.scratch/<feature-slug>/issues/<NN>-<slug>.md`
- 工单编号从 `01` 开始
- 每张工单在文件顶部附近使用 `Status:` 记录状态
- 讨论记录追加到 `## Comments` 章节

## 发布工单

在 `.scratch/` 下创建对应的功能目录和 Markdown 文件。

## 获取工单

读取用户指定的工单路径或编号。

## Wayfinder 操作约定

- 决策地图：`.scratch/<effort>/map.md`
- 子工单：`.scratch/<effort>/issues/<NN>-<slug>.md`
- 工单类型：`Type: research|prototype|grilling|task`
- 工单状态：`Status: claimed|resolved`
- 依赖关系：`Blocked by: NN, NN`
- 领取工单：将状态设置为 `Status: claimed`
- 解决工单：添加 `## Answer`，设置 `Status: resolved`，并将结果记录到 `map.md`
