# 开发约定（给 AI 助手和人）

先读 README.md 和 docs/ARCHITECTURE.md。本文件与 AGENTS.md 内容相同，修改时两份一起改。

## 产品原则

- hush.md 是 **Markdown 原生的编辑器**：`.md` 文件就是笔记，没有单独的文档库索引。任何设计都不能让「文件里有什么」和「应用显示什么」脱节。
- 用户选中的文件夹里，只有 `.md` 文件是笔记。不要读取、改写或删除其他文件；`.hush/` 是唯一由应用自己创建的位置。
- 未改动的 Markdown 必须逐字节写回。改动一个段落不能顺手重排整个文件。
- 界面细节以代码里的 `Theme` token 和现有实现为准；和代码对不上时，以代码为准。
- 保留现有元素、顺序、行为和快捷键；视觉使用现有 Theme 语义 token。

## 代码

- **不跑 rustfmt。** 代码是刻意的紧凑风格，新代码照周围代码的风格写，注释密度也一样。
- **文档引擎在 `crates/core`（`hush_core`），不依赖 GPUI。** 模型、Markdown 读写、编辑内核、文件夹库、导入和导出放这里；界面代码放根 crate。
- **改文档结构只经过 `editor::Core`。** 顺序是：`checkpoint` → Core 方法 → `Editor::changed()`。新增结构性操作时，同时在 `fuzz_tests.rs` 的 `step` 里加一种随机操作。
- 能有子级的只有 `BlockKind::nests()` 的 block 和容器（引用、高亮块、分栏、列）。代码块里没有样式和行内对象。
- **Markdown 的读写规则见 docs/FORMAT.md。** 解析器在 `crates/core/src/md.rs`，来源记录在 `source.rs`。改读写逻辑后，`md::tests::real_documents_round_trip` 必须仍然通过；大改时用 `HUSH_MD_ROUNDTRIP_DIR=<文件夹>` 跑一批真实文件。
- 新增块类型时，顺手在 `md.rs` 的测试里加它的规范写法，并确认渲染视图和源码视图都能表示它。
- **颜色用 `Theme::*` 语义 token**，其次是 `theme::craft` 色阶（`c(::N300)`）。只有实测值才写十六进制并注明出处。阴影用 `theme::shadow_*`，菜单外壳用 `ui::popover` / `ui::menu_row`。
- 改了 `vendor/` 的代码，同步更新 `vendor/PATCHES.md`。
- 界面文案同时提供中英文，`settings::tr` / `text` / `localized_format!`；有测试会检查遗漏。

## 提交前

```sh
scripts/check.sh        # cargo test --release + clippy --all-targets 零警告
```

改了编辑内核时，多跑一些随机种子：`FDL_FUZZ_SEEDS=2000 cargo test --release random_edits`。

想每次提交自动检查：`git config core.hooksPath scripts/hooks`。

发布版本时，把两个 `Cargo.toml` 的版本号一起更新。未经用户明确要求，不创建提交。

## 用户数据（绝对不要碰）

- 默认笔记库 `~/Documents/Hush.md` 是使用者的真实数据：不要提交进仓库，不要删改里面的文件。测试一律用临时目录，用 `HUSH_MD_HOME` 指定。
- 别的应用的进程和数据一律不碰；结束进程只能匹配本仓库自己编译出来的二进制。

## 环境注意

- shell 是 zsh：没有匹配的通配符会直接报错，`--include=*.rs` 这类参数要加引号。
- `grep` 实际是 ugrep。
- macOS 的 `sed -i` 要写成 `sed -i ''`。
- 没有 `timeout` 命令。
