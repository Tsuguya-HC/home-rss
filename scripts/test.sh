#!/usr/bin/env bash
# Every test suite in the repo. Automated implementers run this file as the
# whole suite, so a suite left out of it is never checked there. e2e needs
# E2E_DATABASE_URL and fails without it.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

host=$(rustc -vV | awk '/^host:/{print $2}')
cargo test --workspace --target "$host"

(cd ui && pnpm install --frozen-lockfile && pnpm test)

bash e2e/run.sh
