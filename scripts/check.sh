#!/bin/sh
# Before every commit: tests (including a short fuzz run) and clippy, zero warnings.
# Point HUSH_MD_ROUNDTRIP_DIR at a folder of .md files to also check that a
# batch of real Markdown round-trips byte for byte.
# Longer fuzzing: FDL_FUZZ_SEEDS=2000 scripts/check.sh
set -e
cd "$(dirname "$0")/.."
cargo test --release --workspace --locked
cargo clippy --release --workspace --locked --all-targets -- -D warnings
