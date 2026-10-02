#!/usr/bin/env bash
# Changed lines since <base-ref> that no test runs. Runs the Rust unit tests
# and the UI tests with coverage, then reports through repo-checks'
# diff-coverage. Exit 1 means lines were reported; any failure before that
# (a failing test included) is exit 2, so it cannot pass for a report.
set -Eeuo pipefail
trap 'exit 2' ERR
cd "$(git rev-parse --show-toplevel)"

if [ $# -ne 1 ]; then
  echo "usage: scripts/diff-coverage.sh <base-ref>" >&2
  exit 2
fi
base=$1
git rev-parse --verify --quiet "${base}^{commit}" >/dev/null || {
  echo "diff-coverage: not a commit: ${base}" >&2
  exit 2
}

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# Only this branch's own changes, not what the base gained since.
fork=$(git merge-base "$base" HEAD)
diff_opts=(--no-color --no-ext-diff --unified=0 --src-prefix=a/ --dst-prefix=b/)

# Untracked files are not in `git diff`; add them as wholly new.
git diff "${diff_opts[@]}" "$fork" > "$work/diff"
git ls-files --others --exclude-standard -z | while IFS= read -r -d '' f; do
  git diff "${diff_opts[@]}" --no-index /dev/null "$f" >> "$work/diff" || [ $? -eq 1 ]
done

host=$(rustc -vV | awk '/^host:/{print $2}')
cargo llvm-cov --workspace --target "$host" --lcov --output-path "$work/rust.lcov" >&2

(
  cd ui
  pnpm install --frozen-lockfile >&2
  pnpm exec vitest run --coverage.enabled --coverage.reporter=lcov \
    --coverage.reportsDirectory="$work/ui-coverage" >&2
)

cargo build -q -p home-rss-repo-checks --target "$host" --bin diff-coverage >&2

status=0
cargo run -q -p home-rss-repo-checks --target "$host" --bin diff-coverage -- \
  . "$work/diff" "$work/rust.lcov" "$work/ui-coverage/lcov.info" || status=$?
[ "$status" -le 1 ] || status=2
exit "$status"
