#!/usr/bin/env bash
# The engine core does no file, network or process I/O and reads neither the
# environment nor the clock. Resources come from the host's callback, time
# from the host's now_ns, configuration from API parameters (see
# docs/design/p1-embedded.md §2.4). This keeps the core deterministic, and
# content such as url("file:///etc/passwd") unable to read anything.
set -euo pipefail

core="crates/erk-dom/src crates/erk-invalidation/src crates/erk-style/src crates/erk-renderer/src"
for dir in $core; do
  [ -d "$dir" ] || { echo "$dir is missing"; exit 1; }
done

# Code lines only: a comment may explain the rule without breaking it.
code=$(grep -rnv --include='*.rs' '^[[:space:]]*//' $core)

# fontique reads font files itself when asked to scan the system or load
# paths; the host scans and sends the fonts (p1-contract §6.2).
forbidden='std::(fs|net|process|env)\b|std::\{[^}]*\b(fs|net|process|env)\b|\b(fs|net|env)::|\bFile::|\bOpenOptions\b|\bTcp(Stream|Listener)\b|\bUdpSocket\b|\bCommand::new\b|\bInstant::now\b|\bSystemTime\b|\bload_(system_fonts|fonts_from_paths)\b|system_fonts:[[:space:]]*true'

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

# A dependency doing the I/O for the core breaks the rule as well: with its
# `system` feature fontique (directly or through Parley) scans and reads the
# system's fonts. Resolved for the renderer alone, as building it alone
# would: the shell enables the feature for itself.
if ! tree=$(cargo tree -p erk-renderer -e features --target all --locked 2>&1); then
  echo "cargo tree failed:"
  echo "$tree"
  exit 1
fi
if grep -Eq '(fontique|parley) feature "system"' <<< "$tree"; then
  echo "erk-renderer must not enable the system font scan of fontique or parley:"
  grep -E '(fontique|parley) feature "system"' <<< "$tree"
  exit 1
fi
echo "no system font scan in the engine core's dependencies"
