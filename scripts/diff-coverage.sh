#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 1 ]; then
  echo "usage: scripts/diff-coverage.sh <base-ref>" >&2
  exit 1
fi
base_ref=$1
cd "$(git rev-parse --show-toplevel)"

tmp_dir=$(mktemp -d)
trap 'rm -rf "$tmp_dir"' EXIT

host_target=$(rustc -vV | awk '/^host:/{print $2}')
if [ -n "${DIFF_COVERAGE_RUST_LCOV:-}" ]; then
  rust_lcov=$DIFF_COVERAGE_RUST_LCOV
else
  rust_lcov=$tmp_dir/rust.lcov
  cargo llvm-cov --workspace --target "$host_target" --lcov --output-path "$rust_lcov" >&2
fi

if [ -n "${DIFF_COVERAGE_UI_LCOV:-}" ]; then
  ui_lcov=$DIFF_COVERAGE_UI_LCOV
else
  ui_lcov=$tmp_dir/ui-coverage/lcov.info
  (cd ui && pnpm exec vitest run --coverage --coverage.provider=v8 --coverage.reporter=lcov --coverage.reportsDirectory="$tmp_dir/ui-coverage" >&2)
fi

cat >"$tmp_dir/diff_coverage.py" <<'PYEOF'
import re
import sys

# The single place holding the static e2e-only rule: Spin-only APIs that unit
# tests cannot execute on the host. Anything unmatched falls to untested.
SPIN_PATTERNS = ("Connection::open", "variables::get", "http::send", "http_service")

FN_RE = re.compile(r"^\s*(?:pub\s*(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+[A-Za-z_][A-Za-z_0-9]*")
ATTR_RE = re.compile(r"^\s*#\[.*\]\s*$")
RAW_STR_RE = re.compile(r"r(#+)?\"")
UI_CODE_RE = re.compile(r"\.(m|c)?[tj]sx?$")


def parse_lcov(path):
    hits = {}
    current = None
    try:
        with open(path, encoding="utf-8", errors="replace") as handle:
            for raw in handle:
                line = raw.rstrip("\n")
                if line.startswith("SF:"):
                    current = line[3:]
                elif line.startswith("DA:") and current is not None:
                    count = line[3:].split(",", 2)
                    try:
                        number, taken = int(count[0]), int(count[1])
                    except (ValueError, IndexError):
                        continue
                    key = (current, number)
                    hits[key] = max(hits.get(key, 0), taken)
                elif line == "end_of_record":
                    current = None
    except OSError:
        pass
    return hits


def is_covered(hits, path, number):
    for (source, line_number), taken in hits.items():
        if line_number != number:
            continue
        if source == path or source.endswith("/" + path):
            if taken > 0:
                return True
    return False


def strip_line(line, in_block):
    out = []
    i = 0
    n = len(line)
    while i < n:
        if in_block:
            end = line.find("*/", i)
            if end == -1:
                return "", True
            i = end + 2
            in_block = False
        elif line.startswith("//", i):
            break
        elif line.startswith("/*", i):
            in_block = True
            i += 2
        elif line[i] == '"':
            end = i + 1
            while end < n:
                if line[end] == "\\":
                    end += 2
                elif line[end] == '"':
                    break
                else:
                    end += 1
            i = end + 1
        elif line[i] == "'":
            if i + 2 < n and line[i + 2] == "'":
                i += 3
            elif i + 3 < n and line[i + 1] == "\\" and line[i + 3] == "'":
                i += 4
            else:
                out.append(line[i])
                i += 1
        else:
            raw = RAW_STR_RE.match(line, i)
            if raw:
                hashes = raw.group(1) or ""
                end = line.find('"' + hashes, raw.end())
                i = n if end == -1 else end + 1 + len(hashes)
            else:
                out.append(line[i])
                i += 1
    return "".join(out), in_block


def function_ranges(lines):
    stripped = []
    in_block = False
    for line in lines:
        code, in_block = strip_line(line, in_block)
        stripped.append(code)
    ranges = []
    total = len(lines)
    for index, raw in enumerate(lines):
        if not FN_RE.match(raw):
            continue
        start = index
        probe = index - 1
        while probe >= 0 and ATTR_RE.match(lines[probe]):
            start = probe
            probe -= 1
        depth = 0
        opened = False
        end = index
        for cursor in range(index, total):
            for char in stripped[cursor]:
                if char == "{":
                    depth += 1
                    opened = True
                elif char == "}":
                    depth -= 1
            if opened and depth == 0:
                end = cursor
                break
        ranges.append((start, end))
    return ranges


def parse_diff(text):
    changed = {}
    path = None
    number = 0
    for raw in text.splitlines():
        if raw.startswith("+++ "):
            candidate = raw[4:].strip()
            if candidate.startswith("b/"):
                candidate = candidate[2:]
            path = None if candidate == "/dev/null" else candidate
        elif raw.startswith("@@ "):
            number = int(re.search(r"\+(\d+)", raw).group(1))
        elif path is not None and raw.startswith("+"):
            changed.setdefault(path, set()).add(number)
            number += 1
    return changed


def classify_rust_line(path, number):
    with open(path, encoding="utf-8", errors="replace") as handle:
        lines = handle.read().splitlines()
    for start, end in function_ranges(lines):
        if start + 1 <= number <= end + 1:
            body = "\n".join(lines[start : end + 1])
            if any(pattern in body for pattern in SPIN_PATTERNS):
                return "e2e-only"
            break
    return "untested"


def main():
    rust_lcov = parse_lcov(sys.argv[1])
    ui_lcov = parse_lcov(sys.argv[2])
    changed = parse_diff(sys.stdin.read())
    for path in sorted(changed):
        for number in sorted(changed[path]):
            if path.startswith("ui/") and UI_CODE_RE.search(path):
                if not is_covered(ui_lcov, path, number):
                    print(f"{path}:{number}\tuntested")
            elif path.endswith(".rs"):
                if not is_covered(rust_lcov, path, number):
                    print(f"{path}:{number}\t{classify_rust_line(path, number)}")


main()
PYEOF

git diff --no-color -U0 "$base_ref" -- | python3 "$tmp_dir/diff_coverage.py" "$rust_lcov" "$ui_lcov"
