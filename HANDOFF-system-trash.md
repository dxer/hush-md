# Hand-off — 删除走系统回收站 + 库内布局归于 `.hush/`

生成时间：2026-11-05
基线提交：`109bede Baseline before system-trash and library-layout changes`（本次改动前 `git init` 建立的）

> **状态：已完成（未提交）。** 下面第 1 节是已定方案，第 2、3 节保留当时的计划与进度记录；
> 实际落地以工作区代码为准。`scripts/check.sh` 通过，`FDL_FUZZ_SEEDS=2000 cargo test --release random_edits` 通过。
> 剩下没做的只有两件需要额外环境的：`tools/promo/gen_library.py` / `tools/harness/` 仍生成 DocxXML 库，
> 性能数字要等它们改成 Markdown 后再测（见 `docs/PERFORMANCE.md`）。

---

## 1. 已定方案（用户逐项确认过，不要再改回）

| 议题 | 结论 |
|---|---|
| 删除 | 删除 = 移到**系统**回收站（`trash` crate）。应用内不再有回收站。 |
| 删除确认 | **单篇不确认**，直接进系统回收站；**批量删除、删除文件夹**仍然确认。 |
| 无回收站的位置 | 网络盘 / FAT U 盘 / opt-out 挂载：删除失败 → **二次确认后永久删除**（`remove_file` / `remove_dir_all`）。 |
| 回收站页面 | 整体移除：`View::Trash`、设置侧栏入口、`Restore` / `Purge` 确认框、相关文案与 store 空壳。 |
| 笔记扩展名 | `.md` 和 `.markdown` 都算笔记（与 `docs/FORMAT.md` 一致）。 |
| 应用创建的目录 | 图片、附件、主题副本全部收进 `.hush/`：`.hush/assets/`、`.hush/themes/`。**旧 `assets/` 只读兜底**（不迁移、不改写用户笔记）。 |
| 表格单元格 | 格内 Enter = 跳到下一格（复用 `Core::cell_step(true)`）；保存时把多余块压成一行（**不能丢文字**）；编辑后要保住 `:---` 对齐写法。 |
| 性能文档 | 删掉 quiet.md 的旧数字，只留方法和复现步骤。 |

---

## 2. 已完成（未提交）

```
 Cargo.lock                | 163 +++++++++++++++++-
 crates/core/Cargo.toml    |   3 +
 crates/core/src/store.rs  |  84 ++++----
 crates/core/src/vault.rs  |  80 +++++---
 src/editor/file.rs        |   2 +-
 src/editor/image.rs       |   2 +-
 src/themes.rs             |   4 +-
```

### `crates/core/Cargo.toml`
新增 `trash = "5.2"`（MIT，实测在本机可用：`trash::delete()` 后文件离开原位）。

### `crates/core/src/vault.rs`
- 新增 `EXTENSIONS = ["md", "markdown"]`、`is_note_path(&Path)`（大小写不敏感）、`stem_of(rel)`。
- 扫描/`has_note_below`/`notes_in`/`walk` 全部改用 `is_note_path`，`walk` 的 stem 改用 `stem_of`。
- 新增 `NoTrash(String)` 错误类型（`Display` + `std::error::Error`），表示该卷没有回收站。
- `delete()` → `trash::delete()`，失败包成 `NoTrash`；新增 `delete_permanently()`。
- `delete_folder()` → `trash::delete()`；新增 `delete_folder_permanently()`。
- 文件头注释重写（说明删除走系统回收站、`.hush/` 是应用唯一创建位置）。

### `crates/core/src/store.rs`
- `assets_dir()` → `<vault>/.hush/assets`；新增 `legacy_assets_dir()` → `<vault>/assets`。
- `image_path_in()` 末尾增加兜底：相对路径 `assets/…` 找不到时再试旧的 `assets/`。
- 空库判断改用 `vault::is_note_path`（不再只认 `.md` / 写死 `.markdown`）。
- `delete(id)`（去掉无用的 `_root` 参数）、新增 `delete_permanently(id)`，共用私有 `forget(id)`；`forget` 现在也清 `search` 缓存。
- `delete_folder(rel)` / 新增 `delete_folder_permanently(rel)`，共用 `forget_folder(rel)`。
- **删除** `Trashed`、`TrashEntry`、`TRASH_DAYS`（以及 `list_trash` / `restore` / `purge` 三个空壳）。

### 其它
- `src/themes.rs`：`dir()` → `<vault>/.hush/themes`，模块注释同步。
- `src/editor/file.rs` / `src/editor/image.rs`：注释里的路径更新为 `.hush/assets/...`。

### 当前编译状态
- `cargo check -p hush_core` ✅ 通过。
- `cargo check --bin hush_md` ❌ **7 个错误**，全部是「上层还没跟上」：

```
src/trash.rs:8       unresolved import TRASH_DAYS, TrashEntry
src/app.rs:265       TrashEntry 不存在
src/shared_store.rs:29  delete 现在只收 1 个参数
src/shared_store.rs:30  list_trash 不存在（+ TrashEntry）
src/shared_store.rs:31  restore 不存在
src/shared_store.rs:32  purge 不存在
```

- `Cargo.lock` 已更新（新增 `trash`、`chrono`、`urlencoding`、`windows 0.62` 链）。**`scripts/check.sh` 用 `--locked`，所以 lock 必须一起提交。**

---

## 3. 待办（按建议顺序）

### 3.1 `src/trash.rs` 拆解（解开大部分错误）
这个文件混了两类东西，只有页面部分该删：

**保留**（被 home / modes / transfer / search / docinfo / tree 引用）：
- `page_header()`（依赖 `Workspace::header_navigation`）
- `empty_state()`
- 那行再导出：`pub(crate) use crate::ui::{close_button, dialog, list_header_text, list_line, menu_label, primary_button, row_hover, scrim, secondary_button};`
- `render_confirm()`（要进一步改，见 3.2）

**删除**：`TRASH_MENU_W`、`remaining()`、`trash_location()`、`render_trash()`、`render_trash_menu()`、`restore_doc()`、`purge_doc()`、`show_store_warnings()`（若无其它引用），以及 `Confirm::Restore` / `Confirm::Purge` 两个分支。

**保留下来的确认框文案要改**（`render_confirm` 里 `Confirm::Delete` / `Confirm::DeleteFolder` 两处）：
- 现在是「删除的内容将进入回收站，30 天后自动彻底删除」——**30 天是应用自己回收站的承诺，必须删掉**；系统回收站何时清空由系统决定。
- 改成「文件会移到系统回收站，可在访达 / 资源管理器中恢复」/「Moved to the system trash; restore it from Finder or Explorer.」。
- `DeleteFolder` 的「文件夹和其中的笔记会一起删除，无法撤销」也要改口径（现在真的可恢复，不再是「无法撤销」）。
- 新增 `Confirm::DeletePermanent` 的文案：「该位置不支持回收站，删除后无法恢复」/「This location has no trash. Deleting is permanent.」——这是二次确认，语气必须明确。

**顺带**：`settings.rs:909` 那条 `删除的内容将进入回收站，30 天后自动彻底删除` 的英文翻译要一起改，别只改中文。

**建议做法**：把文件改名为 `src/dialogs.rs`、`main.rs` 里 `mod trash;` → `mod dialogs;`，然后把下列文件的 `crate::trash::` 改成 `crate::dialogs::`：
`home.rs:18`、`modes.rs:6,184`、`transfer.rs:11`、`search.rs:10`、`docinfo.rs:141,158,211`、`tree.rs:11`。
（`ui.rs` 不 import `Workspace`，所以 `page_header` 不能简单搬进 `ui.rs`。若要彻底去掉「trash」这个名字，可把 `page_header` 挪到 `app.rs`。）

### 3.2 `src/app.rs`
- 删字段 `pub trash`（:265，及 :439 初始化）、`pub trash_menu`（:266，及 :440）。
- 删 `View::Trash`（:172）以及所有分支：`:735-737`、`:779-781`、`:1499`（`matches!(self.view, View::Settings | View::Trash)`）、`:1507`（`render_trash_menu`）、`:1537`（`View::Trash => self.render_trash(cx)`）、`:1645`（`.children(trash_menu)`）。
- `delete_docs()`（`:670`）：
  - `self.store.delete(id, root.as_deref())` → `self.store.delete(id)`（`root` 的子树分组逻辑可保留，只是参数不再传）。
  - 失败时先判断是否为 `vault::NoTrash`（`e.downcast_ref::<vault::NoTrash>()`）：是则记下这些 id，弹 `Confirm::DeletePermanent { ids, folder: None }`；否则照旧报错。
- 新增确认项（`enum Confirm`，:178）：
  ```rust
  /// 该系统位置没有回收站：确认后永久删除。
  DeletePermanent { ids: Vec<String>, folder: Option<String> },
  ```
  以及执行分支：`ids` 逐个 `store.delete_permanently`，`folder` 走 `store.delete_folder_permanently`。
- `ask_delete()`（:661）：**单篇删除不再弹框**，直接调用 `delete_docs(&[id])`（需要 `window`，函数签名要跟着改；调用点见 `sidebar.rs:1050`、`docmenu.rs:219`，两处都在 `cx.listener` 里，能拿到 `window`）。
- `delete_folder()`（:1201）：同样把 `NoTrash` 转成 `Confirm::DeletePermanent { folder: Some(path) }`。
- 成功提示「已删除」→ 改成「已移到回收站 / Moved to Trash」。

### 3.3 `src/shared_store.rs`
- `delete(&mut self, id)`（去掉 `root`）。
- 加 `delete_permanently`、`delete_folder_permanently`。
- 删 `list_trash` / `restore` / `purge`。

### 3.4 `src/settings.rs` 与 extensions 的收尾
- **`home.rs:411` / `tree.rs:160` 的 stem 截取要换**：两处都是
  `trim_end_matches(".md").trim_end_matches(".markdown")`。这个写法顺序依赖、大小写敏感，`.markdown` 文件 `note.markdown` 会被先 trim `.md` 失败再 trim `.markdown` 侥幸正确，但 `Note.MD` 会切不掉。改用已加好的 `hush_core::vault::stem_of(&id)`（大小写不敏感）。这两处决定「子页面列表」和主页缩略图按同名文件夹取子笔记，改错会让 `.markdown` 笔记的子页列表空掉。
- `NAV`（:158）里 `None` 那一项是回收站，删掉，数组长度 6 → 5，同时改 `render_settings_sidebar` 的 `page.is_none()` 分支、`in_trash`（:482）、`:485`、`:492`。
- 清理不再使用的翻译键（约 15 条）：`回收站`、`移到回收站`、`清空回收站`、`回收站暂无内容`、`即将彻底删除`、`确认恢复？`、`确认后将恢复至原位置`、`彻底删除`、`彻底删除后…`、`恢复成功`、`恢复成功。部分文档…`、`剩余时间`、`位置`、`{} 天`，以及 `diagnostic_for` 表里 `:1134-1146` 那组「回收站索引」条目。
  ⚠️ 注意 `localization_tests::ui_keys_and_menu_tables_have_english_translations` 会扫源码，删了用法才能删键，反之会报缺翻译。
- **顺带修现有的红测试**：`src/sidebar.rs:903` 用了 `settings::text("文件夹名")`，`localize()` 里没有对应条目 → 补 `"文件夹名" => "Folder name"`（或改为 `tr("文件夹名", "Folder name")`）。

### 3.5 表格（`crates/core`，两处）
1. **格内 Enter 跳下一格**：`src/editor/view.rs:1655` 的 `on_enter` 在 `core.enter()` 之前加分支——光标在表格单元格内时调用 `core.cell_step(true)` 而不是 `enter()`（可参考 `on_tab` 的写法，:1708）。`cell_step` 已处理「最后一格时自动加行」。
2. **保存不丢文字 + 保住对齐**：
   - `crates/core/src/md.rs` 表格序列化（`:788-804`）只写 `cell.blocks.first()`。改为把单元格内所有块的内联文本拼成一行（用空格连接；块内换行按需转 `" "`），空块跳过。
   - 同样位置的分隔行固定写 ` --- `，丢失 `:---` / `---:` / `:---:`。需要在 `model::Table` 上加对齐字段（如 `pub aligns: Vec<Align>`），解析时从 `is_table_delim` 那行读出，序列化时写回；`md.rs:556`、`paste.rs:741,1089`、`markdown.rs:303`、`editor/table_ops.rs:348,366`、`view.rs:946`、`model.rs:1128`、`home.rs:77`、`docxml.rs:482` 所有构造点都要补默认值。老表格（无对齐信息）写 `---`。
   - 补测试：`md::tests` 里加「格内多块压成一行不丢字」和「编辑一列后对齐写法仍在」；往返测试 `real_documents_round_trip` 必须继续通过。

### 3.6 收尾
- `scripts/check.sh` 目前另外有两个 clippy 错误（本次改动前就有）：
  - `src/editor/find.rs:53` collapsible-if（`if self.source { if let Some(ed) = &self.src_editor { … } }` → `if self.source && let Some(ed) = &self.src_editor { … }`）
  - `src/tree.rs:53` unnecessary-sort-by（`v.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()))` → `v.sort_by_key(|a| a.0.to_lowercase())`）
- 测试更新：`crates/core/src/store.rs:639` 的 `s.delete(&list[0].id, None)` 要改成 `s.delete(&list[0].id)`（新签名去掉 `root` 参数）。**注意**：`delete` 现在走系统回收站，测试里删的是临时的 `HUSH_MD_HOME` 目录，会真的往本机回收站塞文件；建议这条测试改用 `delete_permanently`，或直接改为断言「文件已不在原位」。
- 新增测试建议：
  - `.markdown` 文件出现在列表里（`vault::tests::a_folder_of_markdown_is_a_vault`（:530）与 `store::tests::markdown_files_are_the_library`（:655）各加一条断言，现有测试只用了 `.md`）。
  - `is_note_path` 大小写（`Note.MD`）。
  - `stem_of`：`a/b.markdown` → `b`、`X.MD` → `X`，且不误伤 `a.b.md`（→ `a.b`）。
  - 旧 `assets/` 兜底：建 `assets/x.png`，`image_path_in(Some("n.md"), "assets/x.png")` 找得到。
  - 删除进系统回收站不可断言（依赖环境），至少保证 `delete()` 失败时文件仍在。
- 文档：
  - `docs/FORMAT.md`：删除说明改为「移到系统回收站」；`.hush/` 下补 `assets/`、`themes/`；明确 `.md` / `.markdown`。
  - `docs/THEMES.md:8-9`：`themes/<id>.css` → `.hush/themes/<id>.css`。
  - `README.md:29`、`CLAUDE.md:8`、`AGENTS.md`（两份同步）：应用创建的位置不止 `.hush/` 已经不准 → 改为「只用 `.hush/`（设置、图片、附件、主题）」。
  - `docs/PERFORMANCE.md`：删掉 quiet.md 的数字表与原始输出，保留环境/方法/复现，注明待测。
  - `CONTRIBUTING.md:17,23`：`fdl_core` → `hush_core`。
  - `docs/ARCHITECTURE.md:94`：`guides.rs` 不存在，欢迎笔记在 `crates/core/src/welcome.rs`；`:94` 的模块地图也应提到回收站已移除。
  - `THIRD_PARTY.md`：跑 `scripts/licences.sh` 重新生成（新增 `trash` 及其依赖）。
- **打包脚本里仍是 quiet.md 的标识**（`CONTRIBUTING.md` 有说明，但没替换）：
  - `scripts/release.sh:30`：`CFBundleIdentifier` 还是 `com.s1dashu.quietmd`。
  - `packaging/windows/quiet-md.iss`：文件名、`AppId` GUID、`SignTool=quietmd` 都还是 quiet.md 的。
  - 这两处不阻塞开发，但真正发布前必须换，建议这次一起记在待办里。
- **fuzz 测试没有覆盖 Markdown 存储路径**（结构性缺口，影响这次的表格改动）：
  - `crates/core/src/editor/fuzz_tests.rs:8` 用 `crate::docxml::{parse, serialize}` 做种子文档和存盘往返，**不是 `md`**。所以「格内多块 → 保存丢字」这类 bug 随机测试抓不到。
  - 建议：`round_trip()` 里改成（或增加）`md::write` → `md::read` → `md::write` 比对。这会让 fuzz 真正走用户实际用的存储格式，也是 3.5 表格改动的回归网。
  - 注意 DocxXML 种子包含 Markdown 表达不了的块（callout、grid、sub-page-list），直接换成 Markdown 种子会缩小覆盖面；更稳的做法是保留 DocxXML 种子做编辑内核测试，另加一组 Markdown 种子 + Markdown 往返。
- **陈旧的模块注释**：`crates/core/src/store.rs:1` 开头还写「one DocxXML file per document plus a small JSON index」——早已不是 DocxXML，顺手改掉。
- **被 `#[allow(dead_code, unused_imports)]` 挂着的模块**：`src/feishu_theme.rs`（284 行）、`src/editor_theme.rs`（62 行）、`src/editor_ui.rs`（37 行）在 `main.rs:29-34` 挂着 allow。它们其实被 `themes.rs` 用到（`TOKENS`、`base_color`、配色回退），不是死代码，只是 allow 掩盖了真实依赖；顺手把 allow 去掉、看编译会不会报错，能确认到底哪些是多余的。

---

## 4. 验证

```sh
cargo check -p hush_core            # 现在已通过
cargo check --bin hush_md           # 目标：0 error
scripts/check.sh                    # cargo test --release --workspace --locked + clippy -D warnings
FDF_FUZZ_SEEDS=2000 cargo test --release random_edits   # 改过编辑内核/表格后
```

提交前记得 `git add Cargo.lock`（`--locked` 会校验）。

---

## 5. 注意事项

- **不要跑 rustfmt**；新代码照周围紧凑风格写（见 `CLAUDE.md`）。
- 用户数据：默认库 `~/Documents/Hush.md` 绝对不能碰；测试用 `HUSH_MD_HOME` 或临时目录。
- 环境：zsh、`grep` 实为 ugrep、没有 `timeout`；`git add -A` 在这个仓库上第一次跑卡住过一次（已重试成功），大提交建议后台跑。
- 本机 `trash` crate 实测通过（Linux）；macOS/Windows 走各自系统 API，未在本机验证。
- 交接时 `crates/core/tests/tmp_probe.rs` 是我临时写的探针，已删除；`crates/core/tests/fixtures/all-blocks.xml` 是原有文件，保留。
