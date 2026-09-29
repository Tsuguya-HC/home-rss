#!/usr/bin/env bash
# Every test suite in the repo. Automated implementers run this file as the
# whole suite, so a suite left out of it is never checked there. e2e needs
# E2E_DATABASE_URL and fails without it.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

host=$(rustc -vV | awk '/^host:/{print $2}')
cargo fmt --all --check
cargo fmt --manifest-path e2e/Cargo.toml --check
# Clippy checks the host build, not wasm32-wasip1, so that it runs in the test
# leaf, which installs no wasm target.
cargo clippy --workspace --all-targets --target "$host" -- -D warnings
cargo clippy --manifest-path e2e/Cargo.toml --all-targets --target "$host" -- -D warnings
cargo test --workspace --target "$host"

(
  cd ui
  pnpm install --frozen-lockfile
  pnpm run typecheck
  pnpm run lint
  pnpm run fmt:check
  pnpm test
)

bash e2e/run.sh
