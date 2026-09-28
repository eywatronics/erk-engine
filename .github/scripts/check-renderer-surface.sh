#!/usr/bin/env bash
# The shell reaches the renderer only through its thread and messages, and
# the messages hold only plain owned data, so that the channel can become
# IPC in M3. See docs/design/p0-architecture.md §2.2.
set -euo pipefail

src=crates/erk-renderer/src
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

# 1. The renderer's public surface is exactly this. Every module is private,
#    so an item public elsewhere is reachable only through lib.rs. A change
#    here is a design change: update this list in the same pull request.
expected='pub use messages::{Frame, FromRenderer, ToRenderer};
pub use thread::spawn;
pub fn to_png(&self) -> Option<Vec<u8>> {
pub fn render_html(html: &str, width: u16, height: u16) -> Frame {'
actual=$(grep -E '^[[:space:]]*pub\b' "$src/lib.rs" | sed 's/^[[:space:]]*//')
if [ "$actual" != "$expected" ]; then
  echo "erk-renderer's public surface is not the reviewed one; lib.rs has:"
  echo "$actual"
  fail=1
fi

# 2. Methods and trait impls on the message types add to that surface from
#    any module, so they live in messages.rs, apart from to_png in lib.rs.
impls=$(grep -rnE '^[[:space:]]*impl\b.*\b(Frame|ToRenderer|FromRenderer)\b' "$src" \
  | grep -v "^$src/messages.rs:" | sed -E 's/:[0-9]+:[[:space:]]*/: /' || true)
if [ "$impls" != "$src/lib.rs: impl Frame {" ]; then
  echo "impl blocks for the message types outside messages.rs:"
  echo "$impls"
  fail=1
fi

# 3. messages.rs speaks only in prelude types: no paths (so no engine or
#    third-party type can be named), nothing shared, nothing borrowed.
[ -f "$src/messages.rs" ] || { echo "$src/messages.rs is missing"; exit 1; }
grep -vE '^[[:space:]]*//' "$src/messages.rs" > "${RUNNER_TEMP:-/tmp}/messages.rs.code"
forbid "messages.rs must hold only plain owned data" -nE \
  "::|&'|\b(Arc|Rc|Weak|Box|dyn|Mutex|RwLock|Cell|RefCell|OnceCell|OnceLock|LazyLock|Atomic[A-Za-z0-9]*)\b" \
  "${RUNNER_TEMP:-/tmp}/messages.rs.code"

# 4. The shell never paints synchronously; render_html is for the
#    renderer's own tests.
[ -d crates/erk-shell ] || { echo "crates/erk-shell is missing"; exit 1; }
forbid "the shell talks to the renderer through messages, not render_html" \
  -rn 'render_html' crates/erk-shell

exit $fail
