#!/usr/bin/env bash
# End-to-end suite: build the server and the cleaner, apply migrations/, run both
# under `spin up` against the given PostgreSQL, and run e2e/tests.
#
#   E2E_DATABASE_URL=postgres://user:pass@host:5432/db?sslmode=disable e2e/run.sh
#
# The database comes from outside (a throwaway container locally, a service
# container in CI, a sidecar in the cluster). Its tables are emptied by every test.
set -euo pipefail
cd "$(dirname "$0")/.."

if [ -z "${E2E_DATABASE_URL:-}" ]; then
  echo "E2E_DATABASE_URL is not set. The suite needs a throwaway PostgreSQL and never skips." >&2
  exit 1
fi
# The components may only open postgres://*:5432 (allowed_outbound_hosts), so a
# database on any other port would fail inside the component, not here.
case "${E2E_DATABASE_URL}" in
  *:5432/*) ;;
  *) echo "E2E_DATABASE_URL must use port 5432; the components are only allowed to reach postgres://*:5432" >&2; exit 1 ;;
esac

# A newer CLI can build and run components the cluster nodes refuse, which would
# make this suite green for code that cannot ship. Match the CLI the release uses.
want_spin="$(sed -n 's/^ *SPIN_CLI_VERSION: "\(.*\)"$/\1/p' .github/workflows/_build-rust.yml)"
have_spin="$(spin --version | awk '{print $2}')"
if [ -z "${want_spin}" ] || [ "${have_spin}" != "${want_spin}" ]; then
  echo "spin ${have_spin} is on PATH, the release uses ${want_spin:-<unknown>}" >&2
  exit 1
fi
command -v dbmate >/dev/null || { echo "dbmate is not on PATH" >&2; exit 1; }

dbmate --url "${E2E_DATABASE_URL}" --migrations-dir migrations --no-dump-schema up

cargo build --target wasm32-wasip1 --release -p home-rss-server -p home-rss-cleaner

server_addr="127.0.0.1:${E2E_SERVER_PORT:-38080}"
cleaner_addr="127.0.0.1:${E2E_CLEANER_PORT:-38081}"
pids=()
cleanup() {
  for pid in "${pids[@]}"; do kill "${pid}" 2>/dev/null || true; done
  wait 2>/dev/null || true
}
trap cleanup EXIT

start() {
  local manifest="$1" addr="$2"
  shift 2
  spin up -f "${manifest}" --listen "${addr}" \
    --variable db_url="${E2E_DATABASE_URL}" --variable db_ca_root= "$@" &
  pids+=("$!")
}
start server/spin.toml "${server_addr}"
start cleaner/spin.toml "${cleaner_addr}" --variable retention_days=10

# Probe an unrouted path: Spin answers 404 without running the component, so the
# cleaner does not delete anything while we wait for it.
wait_listening() {
  local addr="$1"
  for _ in $(seq 1 120); do
    if [ "$(curl -s -o /dev/null -w '%{http_code}' "http://${addr}/e2e-probe" || true)" != "000" ]; then
      return 0
    fi
    sleep 0.5
  done
  echo "nothing is listening on ${addr}" >&2
  return 1
}
wait_listening "${server_addr}"
wait_listening "${cleaner_addr}"

host_target="$(rustc -vV | sed -n 's/^host: //p')"
E2E_SERVER_URL="http://${server_addr}" E2E_CLEANER_URL="http://${cleaner_addr}" \
  cargo test --manifest-path e2e/Cargo.toml --target "${host_target}" -- --test-threads=1
