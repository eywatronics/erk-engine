#!/usr/bin/env bash
# erk-style may implement the five `unsafe fn`s Stylo's TElement declares,
# with safe bodies, and nothing else unsafe. Counted over the whole crate
# (src, tests, any build script), not only src.
set -euo pipefail

crate=crates/erk-style
[ -d "$crate" ] || { echo "$crate is missing"; exit 1; }

# Source lines without comments, so a comment mentioning unsafe code does
# not count.
code() {
  grep -rhv '^[[:space:]]*//' "$crate" --include='*.rs' | sed 's://.*$::'
}

# `expect` and an allow with a reason also permit unsafe code under `deny`.
allows=$(grep -rhoE '(allow|expect)\([^)]*\bunsafe_code\b' "$crate" --include='*.rs' | wc -l)
# Every `unsafe` keyword: fn, impl, trait, extern, block, attribute.
tokens=$(code | grep -ow 'unsafe' | wc -l)
fns=$(code | grep -oE '\bunsafe fn\b' | wc -l)

echo "allow/expect(unsafe_code): $allows, unsafe keywords: $tokens, of them unsafe fn: $fns"
if [ "$allows" -ne 5 ] || [ "$tokens" -ne 5 ] || [ "$fns" -ne 5 ]; then
  echo "expected exactly five allowed unsafe fn signatures in erk-style and no other unsafe code"
  exit 1
fi
