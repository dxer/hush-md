# 性能测试

编辑内核（`crates/core` 的 `editor/core.rs`）沿用 quiet.md 的实现，但这套模型在 hush.md 上的性能还没有重新测量。Markdown 的文件读写路径是新的（`md.rs`、`vault.rs`），首次打开一个大文件要解析整份 Markdown，这些开销都要重新计入。

本节只记录测量方法和复现步骤；测出数据后补一张结果表（P50 / P95 / 最大，以及原始输出），并注明日期、提交和机器。

## 环境

测量时请记下：

| 项 | 值 |
|---|---|
| 日期 | 待测 |
| 源码 | 待测（`cargo build --release --locked`） |
| 机器 | 待测 |
| 窗口 | 1280 × 800 pt，2× 缩放，Craft 主题 |
| 文档 | [样例生成器](../tools/promo/gen_library.py) 生成的约 100 万字符、500 多章，含段落、列表、表格和代码 |

## 方法

[harness](../tools/harness/README.md) 回放 [readme-performance.txt](../tools/harness/examples/readme-performance.txt)：打开百万字文档，在首段末尾依次插入 100 个 `x`，随后退格 100 次，再向下滚动 100 次。

`bench` 每发送一次操作后同步调用 `window.draw`，记录这一帧的处理耗时（日志里的 `frame`）。P50 为中位数，P95 为第 95 百分位。它统计应用内的帧处理时间，不包含屏幕呈现延迟，不等同于键盘到屏幕的端到端延迟。这是单机实测，不代表所有设备，也不是和其他应用的对比。

## 复现

测试只使用临时文档库，不会碰 `~/Documents/Hush.md`。

⚠️ `tools/promo/gen_library.py` 和 `tools/harness/` 目前还是 quiet.md 的版本：生成的是 DocxXML 库（`docs/*.xml`），harness 也按 `QUIET_MD_HOME` 和 `quiet_md` 二进制找文件。测量前要先让它们写出 Markdown 库、并指向 `hush_md`（`FDL_BIN`）。

```sh
cargo build --release --locked
python3 tools/promo/gen_library.py /tmp/hush-perf/library   # 需先改成输出 .md
FDL_BIN=target/release/hush_md tools/harness/run.py tools/harness/examples/readme-performance.txt /tmp/hush-perf/library /tmp/hush-perf/run
grep -a bench /tmp/hush-perf/run/log
```

把 `hush-perf/run/log` 里的 `bench` 行和 P50 / P95 / 最大一起记到上面的结果表里。
