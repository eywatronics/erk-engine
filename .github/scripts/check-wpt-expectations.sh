#!/usr/bin/env bash
# WPT results only go down with a written reason.
#
# Compares tests/wpt/expectations.txt with its version at the base commit
# ($1). A test that was PASS and is now anything else must carry
# `# lowered: <reason>` on its line. A PASS line may disappear only when the
# pinned WPT commit changed (the test may be gone upstream). `erk-wpt check`
# already fails when the file and the results disagree; this keeps a
# regression from being recorded silently.
set -euo pipefail

base="${1:-}"
file=tests/wpt/expectations.txt
commit=tests/wpt/WPT_COMMIT

if [ -z "$base" ] || ! git cat-file -e "$base:$file" 2>/dev/null; then
  echo "no earlier WPT expectations at '$base' to compare with"
  exit 0
fi
[ -f "$file" ] || { echo "$file is missing"; exit 1; }

commit_changed=0
if [ "$(git show "$base:$commit" 2>/dev/null || true)" != "$(cat "$commit")" ]; then
  commit_changed=1
fi

# test -> status and test -> reason in the current file.
declare -A status=() reason=()
while IFS= read -r line; do
  case $line in '#'*|'') continue ;; esac
  entry=${line%%#*}
  read -r name state _ <<< "$entry"
  status[$name]=$state
  if [[ $line == *'# lowered:'* ]]; then
    why=${line#*# lowered:}
    why=$(sed 's/^[[:space:]]*//; s/[[:space:]]*$//' <<< "$why")
    reason[$name]=$why
  fi
done < "$file"

fail=0
while read -r name old; do
  [ "$old" = PASS ] || continue
  new=${status[$name]:-}
  if [ -z "$new" ]; then
    if [ "$commit_changed" = 0 ]; then
      echo "$name: a passing test was removed while the WPT commit stayed the same"
      fail=1
    fi
    continue
  fi
  if [ "$new" != PASS ]; then
    if [ -z "${reason[$name]:-}" ]; then
      echo "$name: PASS -> $new without '# lowered: reason'"
      fail=1
    else
      echo "$name: PASS -> $new: ${reason[$name]}"
    fi
  fi
done < <(git show "$base:$file" | sed 's/#.*//' | awk 'NF >= 2 {print $1, $2}')
exit $fail
