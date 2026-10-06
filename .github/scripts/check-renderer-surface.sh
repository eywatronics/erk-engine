#!/usr/bin/env bash
# The renderer's surface is its engine, its raster and plain data: what
# crosses to the host (messages.rs) and to the raster thread (list.rs)
# holds only plain owned data, so that it can cross the C ABI and, should
# the raster ever need it, a process boundary (p1-contract §1.1). The
# embedding layer, erk, is its one user; the shell reaches it through erk
# (checked by the dependency step in CI).
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
expected='pub use engine::Engine;
pub use gpu::Window;
pub use list::Prepared;
pub use messages::{ BoxModel, Cursor, ElementBox, Event, EventKind, FontCatalog, Frame, GenericFamilies, Key, KeyInput, KeyState, Modifiers, NodeKind, Painted, PointerButton, PointerInput, PointerKind, Raster, ResourceKind, ResourceRequest, ResourceResponse, ScriptFallback, Stage, Status, TextBox, };
pub use raster::RasterThread;
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
impls=$(grep -rnE '^[[:space:]]*impl\b.*\b(Frame|Painted|Event)\b' "$src" \
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

# 4. What crosses to the raster (p1-contract §1.1) is plain data too: the
#    display list and the table updates in list.rs name no other type than
#    the prelude's and their own, so no font or image can ride along shared.
#    Without a path (`::`) the file can name nothing from elsewhere, not even
#    through a `use`.
[ -f "$src/list.rs" ] || { echo "$src/list.rs is missing"; exit 1; }
grep -vE '^[[:space:]]*//' "$src/list.rs" > "${RUNNER_TEMP:-/tmp}/list.rs.code"
forbid "list.rs must hold only plain owned data" -nE \
  "::|&'|\b(Arc|Rc|Weak|Box|dyn|Mutex|RwLock|Cell|RefCell|OnceCell|OnceLock|LazyLock|Atomic[A-Za-z0-9]*)\b" \
  "${RUNNER_TEMP:-/tmp}/list.rs.code"
# ...and the types the raster takes are defined there and nowhere else: one
# moved out of list.rs would escape the check above.
forbid "the display list's types belong in list.rs" -rnE --exclude=list.rs \
  '\b(struct|enum|type)[[:space:]]+(DisplayList|DisplayItem|GlyphRun|PositionedGlyph|TableUpdate|FontId|ImageId|Radii)\b' \
  "$src"

# 5. The embedding layer paints through the engine and the raster thread;
#    render_html and its kin are for the renderer's own tests.
for crate in crates/erk crates/erk-shell; do
  [ -d "$crate" ] || { echo "$crate is missing"; exit 1; }
done
forbid "erk paints through its engine and raster, not render_html" \
  -rnE 'render_html|paint_repeatedly|element_boxes|text_boxes' crates/erk crates/erk-shell

exit $fail
