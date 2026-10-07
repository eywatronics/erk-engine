#!/usr/bin/env bash
# Erk never runs scripts (p1-embedded §3.3): `<script>` is parsed and stays
# inert, and no JavaScript engine is ever part of Erk. JavaScript and
# TypeScript drive Erk from outside, as Python and Go do: the engine is the
# host's (Node.js, Bun), never Erk's.
#
# Read from the lock files, not from `cargo tree`: a lock file resolves
# every feature, every target and the dev-dependencies, so an optional, a
# platform-only or a test-only engine is there too, and a renamed
# dependency is listed under its package's own name. Both workspaces: the
# engine's and the fuzz targets'.
set -euo pipefail

locks=(Cargo.lock fuzz/Cargo.lock)
# JavaScript engines and their bindings on crates.io, by package name.
engines='^(boa_[a-z_]+|rquickjs(-[a-z]+)?|quickjs[a-z_-]*|libquickjs[a-z_-]*|qjs[a-z_-]*|v8|rusty_v8|deno_[a-z_]+|mozjs(_sys)?|spidermonkey[a-z_-]*|javascriptcore[a-z_-]*|rusty_jsc[a-z_-]*|jsc[a-z_-]*-sys|duktape[a-z_-]*|ducc[a-z_-]*|hermes[a-z_-]*|nova_vm|starlight|jerryscript[a-z_-]*)$'

fail=0
for lock in "${locks[@]}"; do
  [ -f "$lock" ] || { echo "$lock is missing"; exit 1; }
  names=$(awk -F'"' '/^name = "/ { print $2 }' "$lock")
  [ -n "$names" ] || { echo "found no packages in $lock; the parse is broken"; exit 1; }
  found=$(grep -E "$engines" <<< "$names" || true)
  if [ -n "$found" ]; then
    echo "$lock has a JavaScript engine (p1-embedded §3.3):"
    sed 's/^/  /' <<< "$found"
    fail=1
  fi
done
exit $fail
