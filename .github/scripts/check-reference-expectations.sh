#!/usr/bin/env bash
# Reference-test expectations only go down with a written reason.
#
# Compares crates/erk-renderer/tests/reference/expectations.txt with its
# version at the base commit ($1). A lowered score must carry
# `# lowered: <reason>` on its line; a removed score is allowed only when
# its page's Chrome reference is gone too.
set -euo pipefail

base="${1:-}"
file=crates/erk-renderer/tests/reference/expectations.txt
chrome=crates/erk-renderer/tests/reference/chrome

if [ -z "$base" ] || ! git cat-file -e "$base:$file" 2>/dev/null; then
  echo "no earlier expectations at '$base' to compare with"
  exit 0
fi

fail=0
while read -r name old; do
  line=$(grep -E "^$name[[:space:]]" "$file" || true)
  if [ -z "$line" ]; then
    if [ -f "$chrome/$name.png" ]; then
      echo "$name: expectation removed while its Chrome reference is still there"
      fail=1
    fi
    continue
  fi
  new=$(echo "$line" | sed 's/#.*//' | awk '{print $2}')
  if awk -v a="$new" -v b="$old" 'BEGIN { exit !(a < b) }'; then
    if echo "$line" | grep -q '# lowered:'; then
      echo "$name: lowered from $old to $new, with a reason"
    else
      echo "$name: lowered from $old to $new without '# lowered: reason'"
      fail=1
    fi
  fi
done < <(git show "$base:$file" | sed 's/#.*//' | awk 'NF >= 2 {print $1, $2}')
exit $fail
