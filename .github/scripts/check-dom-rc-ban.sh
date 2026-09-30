#!/usr/bin/env bash
# Reference counting stays out of erk-dom: nodes are owned by the arena and
# addressed by NodeId (docs/design/p0-architecture.md §5). The ban lives in
# crates/erk-dom/clippy.toml; this checks that nothing switches it off.
set -euo pipefail

fail=0

# forbid <message> <grep arguments...>: fail on a match, and also when grep
# itself fails (a missing path must not pass as "no match").
forbid() {
  local message=$1 rc=0
  shift
  grep "$@" || rc=$?
  case $rc in
    0) echo "$message"; fail=1 ;;
    1) ;;
    *) echo "grep failed with $rc: $*"; fail=1 ;;
  esac
}

# 1. No attribute silences the lint, by name or through a group that
#    contains it (disallowed_types is in clippy::style, so clippy::all), or
#    all warnings at once. Module-level attributes are why the canary below
#    is not enough on its own.
forbid "do not silence clippy::disallowed_types in erk-dom" -rnE \
  '#!?\[(allow|expect)\(([^]]*[^a-z_:])?(clippy::(disallowed_types|style|all)|warnings)\b' \
  crates/erk-dom

# 2. Canary: the ban must actually fire. This also catches a deleted or
#    emptied clippy.toml and an allow in the workspace lint table, which
#    no attribute search can see.
lib=crates/erk-dom/src/lib.rs
backup=$(mktemp)
cp "$lib" "$backup"
trap 'cp "$backup" "$lib"' EXIT
printf '\npub struct GuardCanaryRc(pub std::rc::Rc<u8>);\npub struct GuardCanaryArc(pub std::sync::Arc<u8>);\n' >> "$lib"
out=$(cargo clippy -p erk-dom --locked -- -D warnings 2>&1 || true)
for ty in std::rc::Rc std::sync::Arc; do
  if ! grep -qF "disallowed type \`$ty\`" <<< "$out"; then
    echo "clippy no longer rejects $ty in erk-dom:"
    echo "$out" | tail -20
    fail=1
  fi
done

exit $fail
