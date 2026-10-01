#!/usr/bin/env bash
# The license gate is `cargo deny check licenses` against the allow list in
# deny.toml. The list is not the only way through it: an exception, a
# clarification, a skipped or excluded crate, a narrowed target or feature
# set, or a copyleft entry would all let a crate in without touching the
# reviewed list. Each of those is a reviewed decision, so this script fails
# on them; allowing one means changing this script in the same PR.
set -euo pipefail

config=${1:-deny.toml}
if [ ! -f "$config" ]; then
  echo "$config is missing: the license gate has nothing to check against"
  exit 1
fi

# Comments do not count, either way.
body=$(sed -e 's/#.*$//' "$config")
failed=0

for key in exceptions clarify private ignore exclude skip skip-tree targets \
  exclude-dev exclude-unpublished no-default-features features; do
  if grep -Eq "(^|[[:space:].{,\[])\"?$key\"?[[:space:]]*(=|\])" <<<"$body"; then
    echo "$config sets '$key': it lets crates past the allow list"
    failed=1
  fi
done

if ! grep -Eq '^[[:space:]]*all-features[[:space:]]*=[[:space:]]*true' <<<"$body"; then
  echo "$config must check all features (all-features = true)"
  failed=1
fi

# GPL, LGPL and AGPL in any version, also inside an expression.
if grep -Eiq '"[^"]*\b(A|L)?GPL' <<<"$body"; then
  echo "$config allows a GPL-family licence"
  failed=1
fi

exit "$failed"
