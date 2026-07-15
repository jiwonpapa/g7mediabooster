#!/usr/bin/env bash
set -euo pipefail

cargo install --locked cargo-nextest --version 0.9.140
cargo install --locked cargo-llvm-cov --version 0.8.7
cargo install --locked cargo-audit --version 0.22.2
cargo install --locked cargo-deny --version 0.20.2
cargo install --locked cargo-fuzz --version 0.13.2
cargo install --locked cargo-cyclonedx --version 0.5.9
