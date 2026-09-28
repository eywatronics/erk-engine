#!/usr/bin/env bash
# Reference-test expectations only go down with a written reason.
#
# Compares crates/erk-renderer/tests/reference/expectations.txt with its
# version at the base commit ($1). A lowered score must carry
# `# lowered: <reason>` on its line, with a reason of its own: a reason
# already on the base line belongs to an earlier lowering. A removed score
# is allowed only when its page's Chrome reference is gone too, and a
# renamed Chrome reference keeps the score of its old name.
set -euo pipefail

base="${1:-}"
file=crates/erk-renderer/tests/reference/expectations.txt
chrome=crates/erk-renderer/tests/reference/chrome

if [ -z "$base" ] || ! git cat-file -e "$base:$file" 2>/dev/null; then
  echo "no earlier expectations at '$base' to compare with"
  exit 0
fi

fail=0

# The reason after `# lowered:`, or nothing.
reason() { sed -n 's/.*# lowered:[[:space:]]*//p' <<< "$1" | sed 's/[[:space:]]*$//'; }
# The line for a page name in expectations text on stdin.
line_of() { grep -E "^$1[[:space:]]" || true; }

# The test reads the first line for a name; a second one would be ignored.
duplicates=$(sed 's/#.*//' "$file" | awk 'NF >= 2 {print $1}' | sort | uniq -d)
if [ -n "$duplicates" ]; then
  echo "more than one expectation for: $duplicates"
  fail=1
fi

# Chrome references renamed since the base, old name -> new name.
declare -A renamed=()
while IFS=$'\t' read -r status old new; do
  case $status in
    R*) renamed[$(basename "$old" .png)]=$(basename "$new" .png) ;;
  esac
done < <(git diff -M --name-status "$base" -- "$chrome")

while read -r name old; do
  current=$name
  line=$(line_of "$name" < "$file")
  if [ -z "$line" ] && [ -n "${renamed[$name]:-}" ]; then
    current=${renamed[$name]}
    line=$(line_of "$current" < "$file")
  fi
  if [ -z "$line" ]; then
    if [ -f "$chrome/$name.png" ]; then
      echo "$name: expectation removed while its Chrome reference is still there"
      fail=1
    fi
    continue
  fi
  new=$(sed 's/#.*//' <<< "$line" | awk '{print $2}')
  if awk -v a="$new" -v b="$old" 'BEGIN { exit !(a < b) }'; then
    why=$(reason "$line")
    was=$(reason "$(git show "$base:$file" | line_of "$name")")
    if [ -z "$why" ]; then
      echo "$current: lowered from $old to $new without '# lowered: reason'"
      fail=1
    elif [ "$why" = "$was" ]; then
      echo "$current: lowered from $old to $new, but '$why' is the reason for an earlier lowering"
      fail=1
    else
      echo "$current: lowered from $old to $new: $why"
    fi
  fi
done < <(git show "$base:$file" | sed 's/#.*//' | awk 'NF >= 2 {print $1, $2}')
exit $fail
