# 参与开发

代码风格和提交约定见 [AGENTS.md](AGENTS.md)（和 CLAUDE.md 内容相同），动手前先读 [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) 和 [docs/FORMAT.md](docs/FORMAT.md)。

## 从源码构建

macOS 需要 Xcode Command Line Tools；Windows 需要 Visual Studio 2022 C++ Build Tools 和 Windows SDK。Rust 版本见 `rust-toolchain.toml`，rustup 会自动选择。

```sh
cargo build --release --locked
./target/release/hush_md                          # 默认用 ~/Documents/Hush.md
HUSH_MD_HOME=/tmp/notes ./target/release/hush_md # 换一个文件夹
```

开发时请用 `HUSH_MD_HOME` 指向临时目录，别拿自己的真实笔记测试。

仓库是 Cargo workspace：`crates/core` 是不依赖 GPUI 的文档引擎（`fdl_core`），根目录是应用本体。GPUI 和 taffy 用的是 `vendor/` 里的本地补丁版本，改动见 [vendor/PATCHES.md](vendor/PATCHES.md)。`target/debug` 会长到十几 GB，日常只用 release 时可以 `cargo clean --profile dev` 清掉。

## 测试

```sh
scripts/check.sh                                        # 测试 + clippy 零警告，提交前跑
cargo test --release -p fdl_core                        # 只测文档引擎，不编 GPUI
FDL_FUZZ_SEEDS=2000 cargo test --release random_edits   # 更多随机编辑，查编辑内核
```

- 编辑内核有一个随机测试：从一篇包含所有块类型的文档出发，随机做选区、输入、结构、样式、表格和撤销，每一步检查模型不变量，每 25 步存盘再读回。
- Markdown 有往返测试：仓库里的文档读入再写出必须逐字节不变；`HUSH_MD_ROUNDTRIP_DIR=<文件夹>` 可以对一批真实的 `.md` 做同样检查。
- 界面截图和自动化脚本见 [tools/harness/README.md](tools/harness/README.md)。

想每次提交自动检查：`git config core.hooksPath scripts/hooks`。

## 文档

| 文档 | 内容 |
|---|---|
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | 分层、编辑流程、不变量、撤销、渲染、模块地图 |
| [docs/FORMAT.md](docs/FORMAT.md) | 文件夹布局、`.hush/`、支持的 Markdown 写法和无损保存规则 |
| [docs/THEMES.md](docs/THEMES.md) | 文档主题的 CSS 写法：选择器、属性、命名颜色 |
| [AGENTS.md](AGENTS.md) | 代码风格、提交前检查、测试约定 |

## 构建分发包

发布前需完成 `scripts/check.sh`，并确认对应提交的 CI 通过。现有的 `scripts/package.sh` / `scripts/release.sh` 和 `packaging/` 是从 quiet.md 带过来的 macOS / Windows 打包脚本，其中的证书、公证和下载地址都还是 quiet.md 的，正式发布前需要替换。
