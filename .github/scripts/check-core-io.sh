#!/usr/bin/env bash
# The engine core does no file, network or process I/O and reads neither the
# environment nor the clock. Resources come from the host's callback, time
# from the host's now_ns, configuration from API parameters (see
# docs/design/p1-embedded.md §2.4). This keeps the core deterministic, and
# content such as url("file:///etc/passwd") unable to read anything.
set -euo pipefail

core="crates/erk-dom/src crates/erk-style/src crates/erk-renderer/src"
for dir in $core; do
  [ -d "$dir" ] || { echo "$dir is missing"; exit 1; }
done

# Code lines only: a comment may explain the rule without breaking it.
code=$(grep -rnv --include='*.rs' '^[[:space:]]*//' $core)

forbidden='std::(fs|net|process|env)\b|std::\{[^}]*\b(fs|net|process|env)\b|\b(fs|net|env)::|\bFile::|\bOpenOptions\b|\bTcp(Stream|Listener)\b|\bUdpSocket\b|\bCommand::new\b|\bInstant::now\b|\bSystemTime\b'

rc=0
matches=$(grep -E "$forbidden" <<< "$code") || rc=$?
case $rc in
  0)
    echo "the engine core must not do I/O or read the environment or clock; the host provides them:"
    echo "$matches"
    exit 1
    ;;
  1) echo "no I/O, environment or clock access in the engine core" ;;
  *) echo "grep failed with $rc"; exit 1 ;;
esac
