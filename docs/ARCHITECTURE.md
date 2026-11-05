# 架构

hush.md 是一个单进程 GPUI 应用。一个文件夹就是笔记库：里面的每个 `.md` 文件是一篇笔记，磁盘上的目录树就是侧栏里的树。

仓库是 Cargo workspace，分两个 crate：

- **`crates/core`（`hush_core`）**：文档引擎，不依赖 GPUI，可以单独编译和测试（`cargo test -p hush_core`）。包括文档模型、Markdown 读写、编辑内核和撤销、文件夹库、导入导出和主题色板。
- **根 crate（`hush_md`）**：GPUI 应用，即窗口、视图、渲染、菜单和浮层。`main.rs` 把 `hush_core` 的模块重新导出到 crate 根，所以应用里写 `crate::model`、`crate::md`。

依赖只能从应用指向 core。

```
main.rs ─ 窗口
  └─ app.rs  Workspace（窗口根视图）
       ├─ store.rs / vault.rs   笔记库：一个文件夹里的 .md 文件              [core]
       ├─ sidebar / home / search / tree / docmenu / docinfo / modes
       └─ editor::Editor（view.rs）   打开的那篇笔记
            └─ editor::Core（core.rs） 文档 + 选区 + 撤销栈                  [core]
```

## 分层

| 层 | 文件 | 职责 | 规则 |
|---|---|---|---|
| 模型 | `model.rs` | `Document` / `Block` / `BlockKind` / `RichText`（文本 + 样式 run） | 纯数据。`BlockKind::nests()` 决定哪些 block 能有子级 |
| 来源 | `source.rs` | 每个 block 在文件里的字节区间、原文、后续空行，以及内容指纹 | 只记录「从哪来、原来长什么样」，不做解析 |
| 格式 | `md.rs` | Markdown ⇄ `Document`：解析、写回、序列化 | 无损：见下文 |
| 编辑内核 | `editor/core.rs`、`editor/table_ops.rs` | 所有改文档的操作：输入、删除、合并、缩进、转换、拖动、表格、折叠、撤销 | 不碰 GPUI，能直接单测和 fuzz |
| 编辑视图 | `editor/view.rs` + 其余 `editor/*` | 键鼠事件 → Core 调用；浮层、菜单、各 block 的交互 | 改文档只经过 Core |
| 渲染 | `editor/render.rs`、`editor/text.rs` | block → GPUI 元素；`text.rs` 是自绘富文本元素 | 只读文档 |
| 库 | `vault.rs`、`store.rs` | 扫描文件夹、读写文件、每篇笔记的设置 | `vault` 管文件，`store` 管设置和 `Entry` 形状 |

## Markdown 如何做到无损

这是和 DocxXML 最大的不同：Markdown 没有固定 schema，任何解析器都可能丢掉它不认识的东西。做法是：

1. **读入时**，每个 block 记下自己的字节区间、原文，以及后面跟着的空行（`source::Source`）。同时对它的**内容**算一个指纹（`model_fingerprint`：文本、样式、类型、子级）。指纹只看内容和结构，不看它是怎么写的——围栏用几个反引号、列表用 `-` 还是 `*`，都不影响指纹。
2. **写回时**，重新算一次指纹。一致就原样输出当初读到的字节（加上它后面的空行）；不一致才从模型重新序列化。
3. **源码视图**编辑的是文件原文，保存时写这份原文本身，不经过模型序列化。

于是：改一个段落，只有那一段变；front matter、HTML 块、`<!-- 注释 -->`、不认识的扩展语法、表格的对齐写法、代码围栏的长短，全都逐字节保留。

回归测试 `md::tests::real_documents_round_trip` 会读入仓库里的文档并逐字节比对；`HUSH_MD_ROUNDTRIP_DIR=<文件夹>` 可以对任意一批 `.md` 做同样的检查。

### 已知的边界

- **列表项之间的空行**归最后一项所有，这是解析器能区分「松散列表」和「列表后面的段落」的成本最低的做法。
- **引用块内部**被重新解析，不保留原来的 `>` 标记写法（内容本身保留）。
- Markdown 没有、模型也表达不了的东西（比如按钮、倒计时、@文档）不会出现在 hush.md 的文件里。

## 编辑流程

一次编辑的顺序是固定的：

1. 视图收到事件，调用 `core.checkpoint(kind)`。它记一个撤销快照，`generation` 加一，`dirty` 置位。连续输入（同一 block、间隔不到 700ms）合成一步。
2. 调用 Core 的操作方法修改 `core.doc` / `core.sel`。
3. 调用 `Editor::changed()`：`core.reveal_caret()`，有改动时发 `EditorEvent::Changed`。
4. `Workspace` 收到 `Changed` 后 `schedule_save`，500ms 后 `flush()` 写盘。写盘失败时改动保持未保存，顶栏显示错误。

Core 的不变量由 `crates/core/src/editor/fuzz_tests.rs` 在每一步随机编辑后检查（选区落点、run 不越界、分栏比例、表格合并格、代码块无样式、撤销重做一致）。

## 源码视图

`Editor.source` 为真时，整份文件在一个 `SourceView`（`src/editor/source_view.rs`）里编辑：一个文本面，等宽，带 Markdown 语法高亮（`highlight::markdown`，代码围栏内的行按自己的语言着色），排版和渲染视图同一列宽。

- **它就是全文。** 输入、回车、跨段选择、`⌘Z` 都作用在文件原文上，不再按 block 切分。回车就是换行，新行自然成为新段落。
- **每次编辑都重新解析。** `SourceView` 发出 `SourceEvent::Changed`，`Editor` 用 `crate::md::read` 重建模型，并存下一份原文（`Editor.docs`）；保存时 `Workspace::flush` 优先写这份原文，不重新序列化。
- **撤销在两边各自独立。** 源码视图有自己的文字快照栈；切过去时对模型做一次 `checkpoint`，所以渲染视图的撤销能回到切换前。
- 切回渲染视图会再解析一次，块的类型（比如把 `plain` 写成 `## heading`）立刻反映出来。

光标落点用 `TextLayout::index_for`，它接受窗口坐标（行从元素原点排布，函数内部会减去）。

## 撤销

每一步是整篇 `Document` 的快照（`Snapshot`），存在 `VecDeque` 里，上限 500 步、约 256MB，文档再长也至少保留 20 步。快照方案简单可靠，代价是长文档每一步要复制整篇。

## 模块地图

文档引擎（`crates/core/src/`）：

| 模块 | 内容 |
|---|---|
| `model.rs` | 文档模型 |
| `source.rs` | block 的磁盘来源、原文、空行、指纹 |
| `md.rs` | Markdown 解析、无损写回、序列化、标题提取 |
| `vault.rs` | 文件夹扫描、读写文件、新建 / 重命名 / 删除、文件名校验；删除走系统回收站 |
| `store.rs` | 在 `vault` 之上提供 `Entry` / `Meta` 和每篇笔记的设置（没有应用内回收站，删除只经过 `vault`；元数据写在应用数据目录） |
| `paste.rs` | HTML / Markdown 粘贴（含从飞书、网页粘贴） |
| `docxml.rs` | DocxXML 读写，只用于和飞书互通，不再是存储格式 |
| `markdown.rs` | 导出 Markdown（另一种选项集） |
| `palette.rs`、`time.rs`、`lang.rs` | 色板、日期换算、代码语言表 |
| `editor/core.rs`、`editor/table_ops.rs`、`editor/fuzz_tests.rs` | 编辑内核、表格操作、随机测试 |

应用顶层（`src/`）：`app.rs`（`Workspace`）、`sidebar.rs` / `tree.rs`、`home.rs`、`search.rs`、`settings.rs`、`theme.rs` / `themes.rs`、`ui.rs` / `input.rs`、`html.rs` / `printing.rs` / `transfer.rs`、`welcome`（首次启动的欢迎笔记，在 `crates/core/src/welcome.rs`）。

编辑器（`src/editor/`）：`view.rs`、`keymap.rs`、`render.rs`、`text.rs`、`overlays.rs`、`table.rs`、`image.rs`、`math.rs`、`mermaid.rs`、`link.rs`、`find.rs` 等。源码视图是 `source_view.rs`；切换与原文保存在 `view.rs` 的 `toggle_source` / `apply_source` / `join_source`，页面在 `render.rs` 的 `render_source`。

## 图表

```` ```mermaid ```` 围栏在渲染视图里画成图：`mermaid.rs` 用 `mermaid-rs-renderer`（纯 Rust，无浏览器）把源码变成 SVG，再用 GPUI 已经链接的 `usvg` / `resvg` 光栅化，最后以 `img(ImageSource::Render(..))` 画出来。

- GPUI 自己的 `svg()` 元素画的是**单色 alpha 遮罩**，所以彩色图表必须走位图这条路。
- `usvg::Options` 默认不带字体，不 `load_system_fonts()` 的话所有标签都是空白。
- 光栅只缓存在内存里（按源码文本 + 缩放比），不写进笔记库：它是派生数据。
- 解析失败（语法错误、不认识的图类型）时退回普通代码块，源码始终可编辑。
- 文件里保存的永远是那段图表源码，`md.rs` 的往返测试覆盖了这一点。

## 打开一个文件

`hush_md 笔记.md`、Finder / 资源管理器里双击，或应用内的「打开文件…」都走 `main.rs` 的 `open_file_window`：一个新窗口，`Workspace::single_file` 为真，没有侧栏、主页和搜索，只有顶栏和这一篇。

- 文件的**所在文件夹**被当作笔记库（`store::set_root`），所以笔记旁边的 `assets/`、相对链接都按 Markdown 的规矩解析。
- 这个选择是进程级的，所以运行中再打开文件会**另起一个进程**，不会把已打开的资料库指到别处。
- 单文件窗口不播种欢迎笔记，也不往文件夹里写任何东西。
- **把 `.md` 拖进窗口是打开，不是插入。** 在已打开的资料库里拖入本库的笔记就直接切过去；拖入别处的文件另开一个窗口；单文件窗口里则换成那个文件。图片和其他文件仍然按原来的方式插入。
- 安装包声明了 `.md` / `.markdown` 的文件类型（macOS `CFBundleDocumentTypes`，Windows `OpenWithProgids`），系统才会把双击交给我们。

## 界面语言

界面支持简体中文与英文，偏好保存在 `Preferences::english`。静态文案用 `settings::text` / `label` 的映射，成对文案用 `settings::tr`，带参数用 `localized_format!`。用户写的笔记内容不是翻译键。

## 和 quiet.md 的关系

hush.md 起步时借鉴了 [quiet.md](https://github.com/s1dashu/quiet-md)（MIT，作者 s1dashu）：界面、块模型和编辑内核来自那里。此后两边独立演进，**本仓库不跟踪 quiet.md 的更新**，没有上游同步脚本或 merge 记录。

- **来自 quiet.md 的部分**：`model.rs` 的块模型、`editor/core.rs` 的编辑内核和撤销栈、表格操作、`theme.rs` / `ui.rs` / `settings.rs` / `home.rs` / `sidebar.rs` 等界面骨架、`docxml.rs`、`feishu_theme.rs`、`palette.rs`，以及 `vendor/` 三个本地补丁（注释仍写 `quiet.md:`）。
- **hush.md 独有的部分**：`md.rs`（Markdown 解析与无损写回）、`source.rs`（逐字节保留策略）、`vault.rs` / `store.rs` 的文件夹库、系统回收站删除、元数据放在应用数据目录的布局。
- **手工回移**：如果 quiet.md 修了编辑内核的 bug，需要自己对照那一个提交改过来，并补一次 `scripts/check.sh`；没有自动机制。

