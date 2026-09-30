#!/usr/bin/env bash
# The release `erk` binary stays within the size budget in
# .github/size-budget.txt, and the budget only goes up with a written
# reason. The budget was set from a measured baseline (M1.0), not a guess.
#
# usage: check-size-budget.sh <binary> [base commit]
set -euo pipefail

binary=${1:?usage: check-size-budget.sh <binary> [base commit]}
base=${2:-}
file=.github/size-budget.txt
[ -f "$binary" ] || { echo "$binary is missing"; exit 1; }
[ -f "$file" ] || { echo "$file is missing"; exit 1; }

# The budget in bytes: the first number on the first non-comment line.
budget_of() { sed 's/#.*//' | grep -oE '[0-9]+' | head -1; }

budget=$(budget_of < "$file" || true)
[ -n "$budget" ] || { echo "$file holds no budget"; exit 1; }
size=$(wc -c < "$binary" | tr -d ' ')
echo "erk release binary: $size bytes, budget $budget bytes ($((size * 100 / budget))% of it)"

fail=0
if [ "$size" -gt "$budget" ]; then
  echo "the binary is over its size budget"
  fail=1
fi

# Raising the budget needs a reason, written on the budget's line.
if [ -n "$base" ] && git cat-file -e "$base:$file" 2>/dev/null; then
  old=$(git show "$base:$file" | budget_of || true)
  if [ -n "$old" ] && [ "$budget" -gt "$old" ]; then
    reason=$(sed -n 's/.*# raised:[[:space:]]*//p' "$file" | head -1)
    was=$(git show "$base:$file" | sed -n 's/.*# raised:[[:space:]]*//p' | head -1)
    if [ -z "$reason" ] || [ "$reason" = "$was" ]; then
      echo "budget raised from $old to $budget without a new '# raised: reason'"
      fail=1
    else
      echo "budget raised from $old to $budget: $reason"
    fi
  fi
fi
exit $fail
