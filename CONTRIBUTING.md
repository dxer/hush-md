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

仓库是 Cargo workspace：`crates/core` 是不依赖 GPUI 的文档引擎（`hush_core`），根目录是应用本体。GPUI 和 taffy 用的是 `vendor/` 里的本地补丁版本，改动见 [vendor/PATCHES.md](vendor/PATCHES.md)。`target/debug` 会长到十几 GB，日常只用 release 时可以 `cargo clean --profile dev` 清掉。

## 测试

```sh
scripts/check.sh                                        # 测试 + clippy 零警告，提交前跑
cargo test --release -p hush_core                        # 只测文档引擎，不编 GPUI
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
| [docs/FORMAT.md](docs/FORMAT.md) | 文件夹布局、应用数据目录、支持的 Markdown 写法和无损保存规则 |
| [docs/THEMES.md](docs/THEMES.md) | 文档主题的 CSS 写法：选择器、属性、命名颜色 |
| [AGENTS.md](AGENTS.md) | 代码风格、提交前检查、测试约定 |

## 美术资源

图像都是脚本画出来的，改样式就改脚本再重跑，不要直接编辑生成的 PNG/JPG：

```sh
python3 tools/branding/gen_icon.py     # 应用图标：app-icon.png / .ico / 安装器 wizard-logo.bmp
python3 tools/covers/gen_covers.py     # 官方图库的 26 张封面和缩略图
```

图标和封面都由 `build.rs` 打进二进制，重跑脚本后 `cargo build` 即可看到效果。

## 构建分发包

发布前需完成 `scripts/check.sh`，并确认对应提交的 CI 通过。

发布二进制在 `[profile.release]` 里 `strip = "symbols"`：符号表原本占 12 MB，去掉后安装包小很多。不要把 `panic` 改成 `abort`——`src/editor/math.rs` 靠 `catch_unwind` 从 katex 的 panic 里恢复。`vendor/gpui/Cargo.toml` 的 `image` 依赖也关掉了默认特性（见 `vendor/PATCHES.md` 第 15 条），剩下的几种格式够用。

`scripts/package.sh` / `scripts/release.sh` 和 `packaging/` 源自 quiet.md，但标识已经全部换成 hush.md：Bundle ID `com.dxer.hushmd`、安装器 `packaging/windows/hush-md.iss`、Windows 安装器的 `AppId` 是新生成的 GUID（沿用 quiet.md 的会被当成同一产品升级，覆盖掉已装的 quiet.md）。

**签名身份不在脚本里写死**，只从钥匙串读：

- macOS：`scripts/package.sh` 自动取钥匙串里第一个 "Developer ID Application"，**并把选中的全名打印出来**。机器上装着多个项目的证书时，用 `CODESIGN_ID="Developer ID Application: 名字 (TEAMID)"` 指定。
- 公证：`NOTARY_PROFILE` 指向 `xcrun notarytool store-credentials` 建好的 profile。
- Windows：`scripts/package-windows.ps1 -CertificateThumbprint <40 位指纹>`；不传就是未签名构建。

上一版本还写着「证书和下载地址指向 quiet.md」，那句已经不成立了——证书取决于你机器上的钥匙串，下载地址在 Release 里。CI 没有任何证书，产物是未签名的。

**CI 的构建产物。** 每次 push 到 `main`（以及手动 `workflow_dispatch`），两个 job 都会把安装包传到这次运行的 Artifacts 里，保留 30 天：

| Artifact | 内容 |
|---|---|
| `hush-md-macos-<sha>` | `hush.md-<version>.dmg` |
| `hush-md-windows-<sha>` | `hush-md-<version>-windows-x64-setup.exe` 和 `.sha256` |

CI 没有签名证书，所以这些包是**未签名、未公证**的，只用来试装和验证构建：macOS 上要右键「打开」，Windows 上会有 SmartScreen 提示。可以直接复用同一个脚本在本地签：

```sh
PACKAGE_UNSIGNED=1 scripts/package.sh      # 不签名，和 CI 一样
scripts/package.sh                          # 自动找钥匙串里的 Developer ID 证书并公证
scripts/package-windows.ps1                 # 未签名安装器；加 -CertificateThumbprint 则签名
```
