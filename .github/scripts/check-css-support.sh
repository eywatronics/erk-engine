#!/usr/bin/env bash
# Every "Supported" row of docs/css-support.md names the test that proves
# it: a test function (`name`) or a test file (`crate/tests/file.rs`), and
# what it names exists. A feature cannot be claimed without a test behind it.
set -euo pipefail

matrix=docs/css-support.md
[ -f "$matrix" ] || { echo "$matrix is missing"; exit 1; }

tests=$(grep -rh -A1 --include='*.rs' '#\[test\]' crates | grep -oE 'fn [a-z0-9_]+' | sed 's/^fn //' | sort -u)
[ -n "$tests" ] || { echo "found no tests at all"; exit 1; }

fail=0
rows=0
while IFS= read -r row; do
  rows=$((rows + 1))
  notes=${row##*| Supported |}
  names=$(grep -oE '`[a-z][a-z0-9_]*`' <<< "$notes" | tr -d '`' || true)
  paths=$(grep -oE '`[A-Za-z0-9_./-]+/[A-Za-z0-9_./-]+\.rs`' <<< "$notes" | tr -d '`' || true)
  if [ -z "$names" ] && [ -z "$paths" ]; then
    echo "names no test: $row"
    fail=1
    continue
  fi
  for name in $names; do
    if ! grep -qx "$name" <<< "$tests"; then
      echo "no test named '$name': $row"
      fail=1
    fi
  done
  for path in $paths; do
    if [ ! -f "crates/$path" ]; then
      echo "no test file crates/$path: $row"
      fail=1
    fi
  done
done < <(grep -F '| Supported |' "$matrix")

[ "$rows" -gt 0 ] || { echo "no Supported rows found in $matrix"; exit 1; }
echo "$rows Supported rows checked"
exit $fail
