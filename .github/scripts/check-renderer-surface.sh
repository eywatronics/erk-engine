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
#    Each public item is read whole, however rustfmt wraps it: a type added
#    to a multi-line `pub use` list must show up here too.
expected='pub use gpu::Window;
pub use messages::{ Cursor, ElementBox, Event, EventKind, FontCatalog, Frame, FromRenderer, GenericFamilies, Key, KeyInput, KeyState, Modifiers, PointerButton, PointerInput, PointerKind, Raster, ResourceKind, ResourceRequest, ResourceResponse, ScriptFallback, Status, TextBox, ToRenderer, };
pub use thread::spawn;
pub use thread::spawn_on_window;
pub fn to_png(&self) -> Option<Vec<u8>> {
pub fn render_html(html: &str, width: u16, height: u16) -> Frame {
pub fn render_html_with_resources( html: &str, width: u16, height: u16, provide: &mut dyn FnMut(&ResourceRequest) -> Option<ResourceResponse>, ) -> Frame {
pub fn render_html_at_scale( html: &str, width: u16, height: u16, scale: f32, provide: &mut dyn FnMut(&ResourceRequest) -> Option<ResourceResponse>, ) -> Frame {
pub fn paint_repeatedly( html: &str, width: u16, height: u16, runs: usize, gpu: bool, frame_done: &mut dyn FnMut(), ) -> Result<String, String> {
pub fn element_boxes( html: &str, width: u16, height: u16, provide: &mut dyn FnMut(&ResourceRequest) -> Option<ResourceResponse>, ) -> Vec<ElementBox> {
pub fn text_boxes( html: &str, width: u16, height: u16, provide: &mut dyn FnMut(&ResourceRequest) -> Option<ResourceResponse>, ) -> Vec<TextBox> {'
actual=$(awk '
  /^[[:space:]]*pub[[:space:]]/ { item = ""; open = 1 }
  open {
    line = $0
    sub(/^[[:space:]]+/, "", line)
    item = item (item == "" ? "" : " ") line
    # A `pub use` list ends at its `;`, an item with a body at its `{`.
    if (item ~ /^pub use/ ? line ~ /;[[:space:]]*$/ : line ~ /[;{][[:space:]]*$/) {
      print item; open = 0
    }
  }' "$src/lib.rs")
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
