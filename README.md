<h1 align="center">hush.md</h1>

<p align="center">
  <b>一个安静、快速的 Markdown 编辑器和阅读器。</b><br>
  Rust + GPUI 原生绘制，没有 WebView。笔记就是你磁盘上的 <code>.md</code> 文件。
</p>

## 是什么

hush.md 打开一个普通文件夹，把里面的 Markdown 文件当作笔记来编辑和阅读。

- **文件就是真源。** 没有数据库，没有导出步骤。文件夹里每一个 `.md` 就是一个笔记，子文件夹就是分组。用 Git、iCloud 或任何同步工具管理都可以。
- **渲染与源码。** 平时看到的是排好版的文档；按 `⌘⇧M` 切到源码，直接编辑带语法高亮的 Markdown。两种视图编辑的是同一份文件。
- **编辑不破坏文件。** 未改动的块按原文逐字节写回，只有你改过的段落才重新生成。front matter、HTML 块、不认识的扩展语法和不常规的空白都会原样保留（见 [docs/FORMAT.md](docs/FORMAT.md)）。
- **快。** 界面由 Rust 和 GPUI 直接用 GPU 绘制。编辑器沿用 quiet.md 的分层模型和虚拟化列表，百万字文档仍然流畅。

## 功能

- **Markdown**：标题（ATX 与 setext）、段落、有序 / 无序 / 待办列表（可嵌套）、引用、代码块（语法高亮）、分割线、HTML 块、管道表格、图片、链接、自动链接、行内代码、加粗 / 斜体 / 删除线。
- **图表**：```` ```mermaid ```` 围栏直接在页面里画成图（流程图、时序图、类图、状态图、ER、甘特、饼图等 23 种），纯 Rust 渲染，不需要 Node.js 或浏览器。
- **编辑**：渲染视图里直接输入，`#`、`-`、`[]`、`>` 等快捷输入和 `/` 斜杠菜单；源码视图里编辑原文。撤销 / 重做、查找替换、块选择、拖动排序。
- **整理**：文件夹树、置顶、收藏、全文搜索、主页。
- **主题**：内置 Craft、飞书、Notion、Obsidian 四套风格，也可以用 CSS 自己写，见 [docs/THEMES.md](docs/THEMES.md)。
- **中英双语界面**，快捷键全部可改，按 `⌘/` 查看。

## 你的文件在哪

默认是 `~/Documents/Hush.md`，可以用环境变量 `HUSH_MD_HOME` 指向任何文件夹。也可以直接把已有的 Markdown 文件夹当作笔记库打开。

保存是原子的：先写临时文件再改名，崩溃不会留下写了一半的笔记。除了你自己的 `.md` / `.markdown`，应用只会在文件夹里创建 `.hush/`（界面设置、每篇笔记的偏好、图片和附件、主题副本）。删除会移到系统回收站，而不是应用内的回收站。

## 从源码构建

需要 Rust 1.99.1（`rust-toolchain.toml` 已固定）。macOS 需要 Xcode Command Line Tools；Windows 需要 Visual Studio 2022 C++ Build Tools 和 Windows SDK；Linux 需要 X11 / Wayland 和 fontconfig 的开发包。

```sh
cargo build --release --locked
./target/release/hush_md
HUSH_MD_HOME=/tmp/notes ./target/release/hush_md   # 指向别的文件夹
```

测试和代码约定见 [CONTRIBUTING.md](CONTRIBUTING.md)，代码结构见 [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)。

## 与其他编辑器的关系

hush.md 起步时借鉴了 [quiet.md](https://github.com/s1dashu/quiet-md)（MIT，作者 s1dashu）：界面、块模型和编辑内核来自那里，再改造成 Markdown 优先的编辑器。这个项目不是 quiet.md 的分支，也不跟踪它的更新，两者各自独立演进。

存储和格式是完全重写的：quiet.md 用飞书 DocxXML 和一个带索引的文档库，hush.md 直接读写你文件夹里的 Markdown 文件，没有索引，删除走系统回收站。`docxml.rs` 留在这里只用于和飞书互通。

## 许可

代码使用 [MIT](LICENSE)。quiet.md 的版权归 s1dashu；第三方代码、字体和素材见 [THIRD_PARTY.md](THIRD_PARTY.md)。
