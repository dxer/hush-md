//! The note a brand-new vault starts with.

/// The welcome note's file name and contents.
pub const TEXT: &str = r#"# 欢迎使用 hush.md

这个文件夹就是你刚打开的笔记库。里面的每个 `.md` 文件都是这里的笔记，用任何编辑器、Git 或同步工具都能打开。

## 两种视图

- 平时是**渲染视图**：Markdown 显示成排好版的文档，可以直接输入、选中、加粗。
- 按 `⌘⇧M` 切到**源码视图**：看到并编辑的就是文件里的原文。

两边改的是同一份文件，随时切换。

## 直接试试

- [ ] 打开源码视图，把下面这行改一改
- [x] 保存是自动的

> 这里是一段引用。

| 你看到的 | 文件里的 |
| --- | --- |
| 标题 | `# 标题` |
| 列表 | `- 列表` |
| 粗体 | `**粗体**` |

```rust
fn main() {
    println!("hello, hush.md");
}
```

## 接下来

- 按 `⌘/` 查看全部快捷键。
- 在左侧新建笔记，或者把已有的 Markdown 文件夹当作笔记库打开（`HUSH_MD_HOME`）。
- 主题在「设置 › 外观」里，也可以自己写 CSS。

编辑器不会重排你没改过的内容：front matter、HTML 块、不认识的语法都会原样保留。
"#;

/// The welcome note's file name, in the interface language.
pub fn file_name(english: bool) -> &'static str {
    if english { "Welcome.md" } else { "欢迎.md" }
}
