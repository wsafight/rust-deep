#!/usr/bin/env bash
# 不依赖宿主架构的质量门禁，适合 Linux/macOS 和 pull request CI。
set -euo pipefail
cd "$(dirname "$0")/.."

cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
cargo test --workspace --doc --locked
mdbook test
mdbook build
bash -n scripts/*.sh tools/*.sh examples/ch08-coherence/cross-crate/run.sh
