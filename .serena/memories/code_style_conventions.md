# 代码风格与约定
- TS/React：组件 PascalCase，hooks `use*`，store 走 Zustand（不使用 persist），持久化必须通过 service -> tauri -> SurrealDB。
- 样式：CSS Modules + Less；支持完整明暗主题，禁止硬编码颜色，优先 CSS 变量。
- i18n：用户可见文本走 i18next。
- Rust：函数 snake_case，结构体 PascalCase，Tauri command 返回 `Result<_, String>`。
- SurrealDB 关键约定：读时用 `type::string(id) as id`，适配层清理 table 前缀 ID；按 ID 查询用 `type::thing('table', $id)`；写入避免 version/revision/id 混入 CONTENT。