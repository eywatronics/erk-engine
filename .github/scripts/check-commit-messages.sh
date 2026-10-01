#!/usr/bin/env bash
# Commit messages, and the pull request title and body, name no AI tool,
# and carry no co-author trailer other than the project's own (see the
# project rules). Checks the commits after the base commit ($1); the
# title and body come from PR_TITLE and PR_BODY when set.
set -euo pipefail

base="${1:-}"
if [ -z "$base" ] || ! git cat-file -e "$base^{commit}" 2>/dev/null; then
  echo "no base commit '$base' to compare with"
  exit 0
fi

text=$(git log --format='%B' "$base..HEAD")
text+=$'\n'"${PR_TITLE:-}"$'\n'"${PR_BODY:-}"
# A body written in GitHub's web editor has CRLF line ends; without this the
# project's own trailer, ending in a carriage return, matches nothing.
text=${text//$'\r'/}
fail=0

names=$(grep -inE 'claude|anthropic|openai|chatgpt|copilot|gemini|generated with' <<< "$text" || true)
if [ -n "$names" ]; then
  echo "commit messages and pull requests name no AI tool:"
  echo "$names"
  fail=1
fi

trailers=$(grep -iE '^[[:space:]]*co-authored-by:' <<< "$text" \
  | grep -vxF 'Co-authored-by: ismet kabatepe <>' || true)
if [ -n "$trailers" ]; then
  echo "the only co-author trailer is 'Co-authored-by: ismet kabatepe <>':"
  echo "$trailers"
  fail=1
fi

exit $fail
