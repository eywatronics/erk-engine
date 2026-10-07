#!/usr/bin/env bash
# The C ABI's two rules that no compiler checks (p1-contract §8, §11):
#
# 1. Every exported function goes through the guard: `guard` (the thread,
#    poison and reentrancy checks and catch_unwind), `guard_free` (the
#    catch_unwind alone, for calls without an app), or `subscribe`, which
#    calls `guard`. A function without it would let a panic cross into C,
#    undefined behaviour, or touch the app from another thread. The one
#    exception is erk_abi_version, which returns a constant.
# 2. erk-ffi is a named unsafe exception: every allow of unsafe code says
#    why, with a `// SAFETY:` comment on its line or in the comment block
#    right above it.
#
# Each check reads a file whole in one awk: a pipeline that stops reading
# early can hang on SIGPIPE in Git Bash on Windows.
set -euo pipefail

src=crates/erk-ffi/src/lib.rs
[ -f "$src" ] || { echo "$src is missing"; exit 1; }
fail=0

# 1. Every exported function's body calls the guard; and subscribe's does.
awk '
  /pub (unsafe )?extern "C" fn [a-z_]+/ {
    match($0, /fn [a-z_]+/); name = substr($0, RSTART + 3, RLENGTH - 3)
    open = 1; guarded = 0; count++
  }
  /^fn subscribe\(/ { name = "subscribe"; open = 1; guarded = 0; inner = 1 }
  open && inner && /guard\(app/ { guarded = 1 }
  open && !inner && /(guard|guard_free|subscribe)\(/ { guarded = 1 }
  open && /^}/ {
    if (!guarded && name != "erk_abi_version") {
      print name " does not go through the guard"; bad = 1
    }
    open = 0; inner = 0
  }
  END {
    if (count < 30) { print "found only " count " exported functions; the parse is broken"; bad = 1 }
    exit bad
  }
' "$src" || fail=1

# 2. Every allow of unsafe code gives its reason, however it is written:
#    alone, with other lints, or as an expect.
for file in crates/erk-ffi/src/*.rs; do
  awk -v file="$file" '
    /(allow|expect)\([^)]*unsafe_code/ && $0 !~ /\/\/ SAFETY:/ && !safe {
      print file ":" NR ": an allow of unsafe code without a SAFETY reason"; bad = 1
    }
    /^[[:space:]]*\/\// {
      if (!comment) { comment = 1; safe = 0 }
      if ($0 ~ /SAFETY:/) { safe = 1 }
      next
    }
    { comment = 0; safe = 0 }
    END { exit bad }
  ' "$file" || fail=1
done

exit $fail
