#!/usr/bin/env bash
# erk-invalidation's one project dependency is erk-dom (p2-incremental
# §3.12): the stages that consume it (style, layout, the display list) must
# not become its dependencies, or the invalidation core would know what it
# serves. Normal and build dependencies, every target and every feature;
# a renamed dependency shows under its package's name. A failing cargo tree
# fails the step.
set -euo pipefail

tree=$(cargo tree -p erk-invalidation -e normal,build --target all --all-features --locked \
  --depth 1 --prefix none)
deps=$(tail -n +2 <<< "$tree" | { grep -E '^erk' || true; } | cut -d' ' -f1 | sort -u)
if [ "$deps" != "erk-dom" ]; then
  echo "erk-invalidation's project dependencies must be erk-dom alone, not:"
  echo "$deps"
  exit 1
fi
