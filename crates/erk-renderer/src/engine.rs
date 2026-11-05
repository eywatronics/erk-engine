//! The engine (p1-contract §1.1): the document, its resources, the user's
//! input and the preparation of each frame, on the caller's thread. Changes
//! apply at once and questions are answered at once; painting is the
//! raster's (raster.rs), which gets a [`Prepared`] frame and nothing else.
//!
//! The embedding layer (`erk`) keeps an engine on its UI thread. A frame's
//! style, layout and display list run on the frame thread, which has a
//! large stack, while the caller waits: layout recurses once per level of
//! nesting, and the deepest document the parser builds needs more stack
//! than a UI thread has (1 MiB on Windows; the measurement is in the M3
//! plan).
//!
//! The frame thread is one for the whole process, started with the first
//! frame and never stopped. Stylo keeps its bloom filter and style sharing
//! cache in thread-local storage and leaks them on purpose, for worker
//! threads that live as long as the process; a thread per frame left
//! ~13 KB behind each frame (found by AddressSanitizer, M3.5). The page and
//! its resources move to the frame thread for the frame and back: moves of
//! a few pointers.

use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::OnceLock;
use std::sync::mpsc::{Sender, channel};
use std::thread::ThreadId;

use erk_dom::NodeId;

use crate::list::Prepared;
use crate::messages::{
    BoxModel, CaretRect, ClipboardRequest, Cursor, Event, FontCatalog, FrameStats, KeyInput,
    NodeKind, PointerInput, ResourceRequest, ResourceResponse, Stage, Status,
};
use crate::page::Page;
use crate::resources::Resources;

/// The stack a frame is prepared on: the deepest document (512 levels)
/// needs 2 to 4 MiB in a release build and 4 to 8 in a debug one. It is
/// address space, committed only as it is used.
pub(crate) const FRAME_STACK: usize = 16 * 1024 * 1024;

/// A frame to prepare on the frame thread: the page and its resources,
/// moved there and back. With `oracle` the page is not prepared but
/// recomputed from nothing, changing nothing ([`Engine::verify`]).
struct FrameJob {
    page: Page,
    resources: Resources,
    width: u16,
    height: u16,
    scale: f32,
    oracle: bool,
    reply: Sender<FrameEvent>,
}

/// What the frame thread prepared.
type FrameOutput = (
    Page,
    Resources,
    crate::list::DisplayList,
    Vec<ResourceRequest>,
);

enum FrameEvent {
    /// A stage ended.
    Mark(Stage),
    /// The frame, or the panic that ended it, and the thread it ran on.
    Done(Box<Result<FrameOutput, Box<dyn Any + Send>>>, ThreadId),
}

/// The frame thread, started with the first frame.
fn frame_thread() -> &'static Sender<FrameJob> {
    static THREAD: OnceLock<Sender<FrameJob>> = OnceLock::new();
    THREAD.get_or_init(|| {
        let (to, jobs) = channel::<FrameJob>();
        std::thread::Builder::new()
            .name("erk-frame".to_owned())
            .stack_size(FRAME_STACK)
            .spawn(move || {
                for job in jobs {
                    let FrameJob {
                        mut page,
                        mut resources,
                        width,
                        height,
                        scale,
                        oracle,
                        reply,
                    } = job;
                    let marks = reply.clone();
                    let done = catch_unwind(AssertUnwindSafe(move || {
                        let (list, requests) = if oracle {
                            let list = page.oracle(width, height, scale, &mut resources);
                            (list, Vec::new())
                        } else {
                            page.prepare(width, height, scale, &mut resources, &mut |stage| {
                                let _ = marks.send(FrameEvent::Mark(stage));
                            })
                        };
                        (page, resources, list, requests)
                    }));
                    let _ = reply.send(FrameEvent::Done(
                        Box::new(done),
                        std::thread::current().id(),
                    ));
                }
            })
            .expect("the frame thread starts");
        to
    })
}

pub struct Engine {
    page: Page,
    resources: Resources,
    /// The viewport in device pixels; no frame is prepared before it is
    /// known.
    size: Option<(u16, u16)>,
    /// Device pixels per CSS pixel.
    scale: f32,
    /// Whether something that shows has changed since the last frame.
    changed: bool,
    /// Whether frames are kept to be checked against the oracle.
    verifying: bool,
    /// The last frame's display list and size, while verifying.
    last: Option<(crate::list::DisplayList, u16, u16)>,
    /// The document as the last frame saw it, and where the journal
    /// missed a change, while verifying.
    seen: Option<erk_invalidation::journal::Snapshot>,
    missed: Option<String>,
    /// How many transactions are open: no frame is prepared until the
    /// outermost closes.
    held: usize,
    /// The thread the last frame was prepared on, for the tests.
    #[cfg(test)]
    frame_thread: Option<ThreadId>,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    /// An engine with an empty document.
    pub fn new() -> Self {
        Self {
            page: Page::parse(""),
            resources: Resources::default(),
            size: None,
            scale: 1.0,
            changed: true,
            verifying: false,
            last: None,
            seen: None,
            missed: None,
            held: 0,
            #[cfg(test)]
            frame_thread: None,
        }
    }

    /// Show `html` instead of the document. The same arena: every id of the
    /// document before goes stale (p1-contract §2), and its resources go.
    pub fn load_html(&mut self, html: &str) {
        self.page.load(html);
        self.resources.new_document();
        self.changed = true;
    }

    /// The viewport is now `width` × `height` device pixels.
    pub fn resize(&mut self, width: u16, height: u16) {
        self.size = Some((width, height));
        self.changed = true;
    }

    /// The screen now has `factor` device pixels per CSS pixel.
    pub fn set_scale(&mut self, factor: f32) {
        self.scale = factor;
        self.changed = true;
    }

    /// The fonts the host can provide (p1-contract §6.2); kept across
    /// documents.
    pub fn set_fonts(&mut self, catalogue: FontCatalog) {
        self.resources.set_fonts(catalogue);
        self.changed = true;
    }

    /// The host's answer to a resource request.
    pub fn complete_resource(&mut self, response: &ResourceResponse) {
        self.resources.complete(response);
        self.changed = true;
    }

    /// The host has no resource for request `id`.
    pub fn resource_missing(&mut self, id: u64) {
        self.resources.missing(id);
        self.changed = true;
    }

    /// Pointer input over the page; what happened that the host may answer.
    pub fn pointer(&mut self, input: &PointerInput) -> Vec<Event> {
        let events = self.restyled_by(|page| page.pointer(input));
        self.changed |= self.page.take_edited();
        events
    }

    /// A key went down or up while the page has the keyboard: its key
    /// event. What the key does by default comes from
    /// [`Engine::key_default`], once the host has seen the event and not
    /// prevented it (M5 plan, decision 7).
    pub fn key(&mut self, input: &KeyInput) -> Option<Event> {
        self.restyled_by(|page| page.key(input))
    }

    /// What `input` does by default: Tab moves the focus, Enter and Space
    /// click the focused link or button. The events that follow from it.
    pub fn key_default(&mut self, input: &KeyInput) -> Vec<Event> {
        let events = self.restyled_by(|page| page.key_default(input));
        // A text field edited or its caret moved (M5.8).
        self.changed |= self.page.take_edited();
        events
    }

    /// What the last key's default asked of the clipboard (Control with C,
    /// X or V in a text field), once: the embedding layer writes the text,
    /// or reads the clipboard and gives it to [`Engine::paste`].
    pub fn take_clipboard(&mut self) -> Option<ClipboardRequest> {
        self.page.take_clipboard()
    }

    /// `text` pasted in the focused text field, at its selection: the
    /// events that follow (an `input` event when it changed the value).
    pub fn paste(&mut self, text: &str) -> Vec<Event> {
        let events = self.page.paste(text);
        self.changed |= self.page.take_edited();
        events
    }

    /// What an input method is composing in the focused text field (M5
    /// plan, decision 6): `text`, shown at the caret and underlined but not
    /// yet in its value, with the caret `caret` bytes into it (at its end
    /// for `None`). Empty text ends the composition. The events that
    /// follow: an `input` event when starting over a selection deleted it.
    /// Nothing in a password field, which takes no composition.
    pub fn compose(&mut self, text: &str, caret: Option<usize>) -> Vec<Event> {
        let events = self.page.compose(text, caret);
        self.changed |= self.page.take_edited();
        events
    }

    /// What an input method composed, committed: the composition goes and
    /// `text` is typed in at the caret, then an `input` event, as a paste.
    pub fn commit_composition(&mut self, text: &str) -> Vec<Event> {
        let events = self.page.commit_composition(text);
        self.changed |= self.page.take_edited();
        events
    }

    /// Whether the focus is in a text field an input method may compose in:
    /// one that is not a password. A window lets the input method work
    /// only then.
    pub fn takes_composition(&self) -> bool {
        self.page.takes_composition()
    }

    /// The host's clock is at `now_ns` (p1-contract §7): what depends on
    /// time, the caret's blink, catches up, and a frame is due if it looks
    /// different.
    pub fn set_time(&mut self, now_ns: u64) {
        self.changed |= self.page.set_time(now_ns);
    }

    /// When the page next looks different with nothing done to it, on the
    /// host's clock: the caret's next blink. `None` while nothing on the
    /// page moves by itself.
    pub fn next_tick(&self) -> Option<u64> {
        self.page.next_tick()
    }

    /// The wheel scrolled by `dx`, `dy` CSS pixels at `x`, `y`.
    pub fn wheel(&mut self, (dx, dy): (f32, f32), (x, y): (f32, f32)) {
        let moved = self.restyled_by(|page| page.wheel((dx, dy), (x, y)));
        self.changed |= moved;
    }

    /// Run `input` on the page; the next frame is painted when what the
    /// user is doing now looks different.
    fn restyled_by<T>(&mut self, input: impl FnOnce(&mut Page) -> T) -> T {
        let before = self.page.interaction();
        let result = input(&mut self.page);
        self.changed |= self.page.shows_change_from(&before);
        result
    }

    /// The topmost node at `x`, `y` (CSS pixels) in the last frame.
    pub fn inspect_at(&self, x: f32, y: f32) -> Option<u64> {
        self.page.hit_test(x, y).map(NodeId::to_bits)
    }

    /// Draw the developer tools' highlight over `node`'s boxes, or nothing.
    pub fn highlight(&mut self, node: Option<u64>) {
        self.page.set_highlight(node);
        self.changed = true;
    }

    /// Whether `node` is a node of the document.
    pub fn contains(&self, node: u64) -> bool {
        self.page.contains(node)
    }

    /// What went wrong since the last call that the host may want to log:
    /// responses refused because they were not what was asked for.
    pub fn take_warnings(&mut self) -> Vec<String> {
        self.resources.take_warnings()
    }

    /// The document node.
    pub fn root(&self) -> u64 {
        self.page.root().to_bits()
    }

    /// The first element inside `scope` (the document for `None`) matching
    /// the CSS selector list `selector`.
    pub fn query(&self, scope: Option<u64>, selector: &str) -> Result<Option<u64>, Status> {
        self.page
            .query(scope, selector)
            .map(|node| node.map(NodeId::to_bits))
    }

    /// Every element inside `scope` (the document for `None`) matching the
    /// CSS selector list `selector`, in document order.
    pub fn query_all(&self, scope: Option<u64>, selector: &str) -> Result<Vec<u64>, Status> {
        Ok(self
            .page
            .query_all(scope, selector)?
            .into_iter()
            .map(NodeId::to_bits)
            .collect())
    }

    /// Set `node`'s text as `textContent` does.
    pub fn set_text(&mut self, node: u64, text: &str) -> Result<(), Status> {
        let result = self.page.set_text(node, text);
        self.changed |= result.is_ok();
        result
    }

    /// A new element named `tag`, not in the document yet; HTML lowercases
    /// the name. `InvalidArgument` for a name that is not one.
    pub fn create_element(&mut self, tag: &str) -> Result<u64, Status> {
        Ok(self.page.create_element(tag)?.to_bits())
    }

    /// A new text node, not in the document yet.
    pub fn create_text(&mut self, text: &str) -> u64 {
        self.page.create_text(text).to_bits()
    }

    /// Insert `child` into `parent`, before `before` or last, moving it from
    /// wherever it was (DOM's `insertBefore`). `InvalidArgument` where DOM
    /// refuses it: a node into itself, the document node, text into the
    /// document, `before` not a child of `parent`.
    pub fn insert(&mut self, parent: u64, child: u64, before: Option<u64>) -> Result<(), Status> {
        self.page.insert(parent, child, before)?;
        self.changed = true;
        Ok(())
    }

    /// Remove `node` and everything in it; their ids go stale.
    pub fn remove(&mut self, node: u64) -> Result<(), Status> {
        self.page.remove(node)?;
        self.changed = true;
        Ok(())
    }

    /// Set attribute `name` of element `node` to `value`.
    pub fn set_attr(&mut self, node: u64, name: &str, value: &str) -> Result<(), Status> {
        self.page.set_attr(node, name, value)?;
        self.changed = true;
        Ok(())
    }

    /// Remove attribute `name` of element `node`; whether it had one.
    pub fn remove_attr(&mut self, node: u64, name: &str) -> Result<bool, Status> {
        let removed = self.page.remove_attr(node, name)?;
        self.changed |= removed;
        Ok(removed)
    }

    /// Attribute `name` of element `node`, if it has one.
    pub fn attr(&self, node: u64, name: &str) -> Result<Option<String>, Status> {
        self.page.attr(node, name)
    }

    /// Element `node`'s value as DOM's `value` reads it: what was set, or
    /// else what the markup says (an `input`'s `value` attribute, a
    /// `textarea`'s text, a `select`'s selected option). `None` for an
    /// element that has no value.
    pub fn value(&self, node: u64) -> Result<Option<String>, Status> {
        self.page.value(node)
    }

    /// Set element `node`'s value as DOM's `value` does: a `select`
    /// selects its first option with that value. It is not an `input`
    /// event: those come from the user. `InvalidArgument` for an element
    /// that has no value.
    pub fn set_value(&mut self, node: u64, value: &str) -> Result<(), Status> {
        self.page.set_value(node, value)?;
        self.changed = true;
        Ok(())
    }

    /// Whether element `node`, a checkbox or radio button, is checked, or
    /// an `option` selected (DOM's `selected`); `None` for any other
    /// element.
    pub fn checked(&self, node: u64) -> Result<Option<bool>, Status> {
        self.page.checked(node)
    }

    /// Check or uncheck element `node`, a checkbox or radio button, or
    /// select or unselect an `option`; a radio button checked unchecks the
    /// others of its group, an option selected in a single-choice list the
    /// other options. `InvalidArgument` for any other element.
    pub fn set_checked(&mut self, node: u64, checked: bool) -> Result<(), Status> {
        self.page.set_checked(node, checked)
    }

    /// Set property `name` of element `node`'s inline style to `value`
    /// (CSSOM's `style.setProperty`); an empty value removes it.
    pub fn set_style_property(&mut self, node: u64, name: &str, value: &str) -> Result<(), Status> {
        self.page.set_style_property(node, name, value)?;
        self.changed = true;
        Ok(())
    }

    /// Remove property `name` from element `node`'s inline style.
    pub fn remove_style_property(&mut self, node: u64, name: &str) -> Result<(), Status> {
        self.page.remove_style_property(node, name)?;
        self.changed = true;
        Ok(())
    }

    /// Property `name` of element `node`'s inline style, if it is set.
    pub fn style_property(&self, node: u64, name: &str) -> Result<Option<String>, Status> {
        self.page.style_property(node, name)
    }

    /// `node`'s text as `textContent` reads it.
    pub fn text(&self, node: u64) -> Result<String, Status> {
        self.page.text(node)
    }

    /// `node`'s parent; the document node has none.
    pub fn parent(&self, node: u64) -> Result<Option<u64>, Status> {
        Ok(self.page.parent(node)?.map(NodeId::to_bits))
    }

    /// `node`'s child at `index`, in document order.
    pub fn child_at(&self, node: u64, index: usize) -> Result<Option<u64>, Status> {
        Ok(self.page.child_at(node, index)?.map(NodeId::to_bits))
    }

    /// How many children `node` has.
    pub fn child_count(&self, node: u64) -> Result<usize, Status> {
        self.page.child_count(node)
    }

    /// What `node` is.
    pub fn kind(&self, node: u64) -> Result<NodeKind, Status> {
        self.page.kind(node)
    }

    /// An element's tag name; `None` for other nodes.
    pub fn tag(&self, node: u64) -> Result<Option<String>, Status> {
        self.page.tag(node)
    }

    /// An element's attributes, in order; none for other nodes.
    pub fn attributes(&self, node: u64) -> Result<Vec<(String, String)>, Status> {
        self.page.attributes(node)
    }

    /// `node`'s box in the last frame, if it has one.
    pub fn node_box(&self, node: u64) -> Result<Option<BoxModel>, Status> {
        self.page.node_box(node)
    }

    /// `node`'s computed style in the last frame, as `name: value;` lines;
    /// `None` for a node without a style.
    pub fn computed_style(&self, node: u64) -> Result<Option<String>, Status> {
        self.page.computed_style(node)
    }

    /// What the pointer should look like over the last frame.
    pub fn cursor(&self) -> Cursor {
        self.page.cursor()
    }

    /// Where the focused text field's caret is on the viewport, as of the
    /// last frame prepared; `None` without one. An input method's
    /// candidate window goes there (M5 plan, decision 6).
    pub fn caret_rect(&self) -> Option<CaretRect> {
        self.page.caret_rect()
    }

    /// Whether a frame should be prepared: something that shows has changed
    /// and the viewport is known.
    pub fn needs_frame(&self) -> bool {
        self.changed && self.size.is_some()
    }

    /// The next frame for the raster, if one is needed, and requests for the
    /// resources the document names that were not asked for yet. Style,
    /// layout and the display list run on a helper thread with
    /// [`FRAME_STACK`]; this waits for it.
    pub fn prepare(&mut self) -> (Option<Prepared>, Vec<ResourceRequest>) {
        self.prepare_marked(&mut |_| {})
    }

    /// [`Engine::prepare`], calling `mark` on this thread as each stage
    /// ends: the embedding layer times the stages, the core reads no clock
    /// (p1-contract §8.1).
    pub fn prepare_marked(
        &mut self,
        mark: &mut dyn FnMut(Stage),
    ) -> (Option<Prepared>, Vec<ResourceRequest>) {
        let Some((width, height)) = self.size.filter(|_| self.changed && self.held == 0) else {
            return (None, Vec::new());
        };
        let actual = self
            .seen
            .as_ref()
            .filter(|_| self.verifying)
            .map(|seen| self.page.difference(seen));
        let (list, requests) = self.on_frame_thread(width, height, false, mark);
        if self.verifying {
            self.last = Some((list.clone(), width, height));
            let found = self.page.changes();
            if let Some(actual) = actual
                && !found.everything
                && *found != actual
                && self.missed.is_none()
            {
                self.missed = Some(format!(
                    "the journal found {found:?} but the document changed {actual:?}"
                ));
            }
            self.seen = Some(self.page.snapshot());
        }
        let updates = self.resources.table_updates(&list);
        self.changed = false;
        let prepared = Prepared {
            updates,
            list,
            width,
            height,
            scale: crate::device_scale(self.scale),
            pending: self.resources.pending(),
        };
        (Some(prepared), requests)
    }

    /// What the last frame did, counted.
    pub fn stats(&self) -> FrameStats {
        self.page.stats()
    }

    /// Keep each frame's display list to check it against the oracle with
    /// [`Engine::verify`]: for tests and fuzzing, since keeping a copy costs.
    pub fn set_verifying(&mut self, verifying: bool) {
        self.verifying = verifying;
        if !verifying {
            self.last = None;
            self.seen = None;
            self.missed = None;
        }
    }

    /// Open a transaction: until the outermost one is committed, no frame
    /// is prepared, so none shows the document halfway through the
    /// changes (M5.1; a host that awaits between changes needs it).
    pub fn begin(&mut self) {
        self.held += 1;
    }

    /// Close the innermost transaction; `InvalidArgument` if none is open.
    pub fn commit(&mut self) -> Result<(), Status> {
        self.held = self.held.checked_sub(1).ok_or(Status::InvalidArgument)?;
        Ok(())
    }

    /// Check the last frame against the oracle: the same document
    /// recomputed from nothing must give the same display list (M5 plan,
    /// decision 9). An error says where they part. Nothing to check before
    /// the first frame; the document must not have changed since the last.
    pub fn verify(&mut self) -> Result<(), String> {
        if !self.verifying {
            return Err("not verifying: call set_verifying(true) first".to_owned());
        }
        if let Some(missed) = self.missed.take() {
            return Err(missed);
        }
        if self.last.is_none() {
            return Ok(());
        }
        if self.changed {
            return Err("the document changed since the last frame".to_owned());
        }
        let Some((last, width, height)) = self.last.take() else {
            return Ok(());
        };
        let (oracle, _) = self.on_frame_thread(width, height, true, &mut |_| {});
        let result = compare(&last, &oracle);
        self.last = Some((last, width, height));
        result
    }

    /// Run the pipeline for the page on the frame thread, with its stack:
    /// the frame, or with `oracle` the recomputation that changes nothing.
    fn on_frame_thread(
        &mut self,
        width: u16,
        height: u16,
        oracle: bool,
        mark: &mut dyn FnMut(Stage),
    ) -> (crate::list::DisplayList, Vec<ResourceRequest>) {
        let (reply, events) = channel();
        let job = FrameJob {
            page: std::mem::replace(&mut self.page, Page::empty()),
            resources: std::mem::take(&mut self.resources),
            width,
            height,
            scale: self.scale,
            oracle,
            reply,
        };
        frame_thread()
            .send(job)
            .unwrap_or_else(|_| panic!("the frame thread stopped"));
        loop {
            match events.recv() {
                Ok(FrameEvent::Mark(stage)) => mark(stage),
                Ok(FrameEvent::Done(done, _thread)) => match *done {
                    Ok((page, resources, list, requests)) => {
                        self.page = page;
                        self.resources = resources;
                        #[cfg(test)]
                        {
                            self.frame_thread = Some(_thread);
                        }
                        return (list, requests);
                    }
                    // The page went with the panic; the empty one stays.
                    Err(panic) => resume_unwind(panic),
                },
                Err(_) => panic!("the frame thread stopped"),
            }
        }
    }
}

/// Where the frame's display list and the oracle's part, if they do.
fn compare(
    frame: &crate::list::DisplayList,
    oracle: &crate::list::DisplayList,
) -> Result<(), String> {
    if frame == oracle {
        return Ok(());
    }
    if frame.canvas != oracle.canvas {
        return Err(format!(
            "the canvas is {:?} in the frame and {:?} in the oracle",
            frame.canvas, oracle.canvas
        ));
    }
    let at = frame
        .items
        .iter()
        .zip(&oracle.items)
        .position(|(a, b)| a != b)
        .unwrap_or(frame.items.len().min(oracle.items.len()));
    Err(format!(
        "the frame ({} items) and the oracle ({} items) part at item {at}:\n  frame:  {:?}\n  oracle: {:?}",
        frame.items.len(),
        oracle.items.len(),
        frame.items.get(at),
        oracle.items.get(at),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::{EventKind, Modifiers, PointerButton, PointerKind};

    /// The stack Windows gives a program's main thread, where a host's UI
    /// thread usually is.
    const MAIN_THREAD_STACK: usize = 1024 * 1024;

    #[test]
    fn a_frame_agrees_with_the_oracle_and_a_change_is_painted_before_it_is_checked() {
        let mut engine = Engine::new();
        engine.load_html("<p id=p>bir</p><p>iki</p>");
        engine.resize(120, 60);
        engine.set_verifying(true);
        assert_eq!(engine.verify(), Ok(()), "nothing to check yet");
        assert!(engine.prepare().0.is_some());
        assert_eq!(engine.verify(), Ok(()));
        let p = engine.query(None, "#p").unwrap().unwrap();
        engine.set_text(p, "üç").unwrap();
        assert!(engine.verify().is_err(), "the change is not painted yet");
        assert!(engine.prepare().0.is_some());
        assert_eq!(engine.verify(), Ok(()));
        engine.set_verifying(false);
        assert!(engine.verify().is_err());
    }

    #[test]
    fn every_kind_of_change_reaches_the_journal_one_frame_at_a_time() {
        let mut engine = Engine::new();
        engine.load_html(
            "<ul id=list><li id=a title=x>bir</li><li id=b>iki</li></ul><p id=p>metin</p>",
        );
        engine.resize(200, 100);
        engine.set_verifying(true);
        assert!(engine.prepare().0.is_some());
        let find = |engine: &Engine, selector| engine.query(None, selector).unwrap().unwrap();
        let (list, a, b, p) = (
            find(&engine, "#list"),
            find(&engine, "#a"),
            find(&engine, "#b"),
            find(&engine, "#p"),
        );
        let text = engine.child_at(p, 0).unwrap().unwrap();
        // Each change in a frame of its own: the journal must find it, and
        // what it finds must be what the document did (`verify`).
        let frame = |engine: &mut Engine, change: &dyn Fn(&mut Engine)| {
            change(engine);
            assert!(engine.prepare().0.is_some());
            assert_eq!(engine.verify(), Ok(()));
            engine.stats().changes
        };
        assert_eq!(
            frame(&mut engine, &|e| e.set_attr(a, "title", "y").unwrap()),
            1
        );
        assert_eq!(
            frame(&mut engine, &|e| assert!(
                e.remove_attr(a, "title").unwrap()
            )),
            1
        );
        assert_eq!(
            frame(&mut engine, &|e| e.set_text(text, "yazı").unwrap()),
            1
        );
        assert_eq!(frame(&mut engine, &|e| e.set_text(b, "üç").unwrap()), 1);
        // A move: the list loses it, the paragraph gains it.
        assert_eq!(frame(&mut engine, &|e| e.insert(p, a, None).unwrap()), 2);
        assert_eq!(frame(&mut engine, &|e| e.remove(a).unwrap()), 1);
        assert_eq!(
            frame(&mut engine, &|e| {
                let made = e.create_element("li").unwrap();
                e.insert(list, made, None).unwrap();
            }),
            1
        );
        // Made and removed in one frame, and set back: nothing.
        assert_eq!(
            frame(&mut engine, &|e| {
                let made = e.create_element("li").unwrap();
                e.insert(list, made, None).unwrap();
                e.remove(made).unwrap();
                e.set_attr(b, "class", "on").unwrap();
                e.remove_attr(b, "class").unwrap();
            }),
            0
        );
    }

    #[test]
    fn a_node_that_joins_the_document_is_new_however_long_it_waited_outside() {
        // Found by the mutation fuzz: a node made in one frame, attached in
        // the next and changed after that was taken for an old node whose
        // attributes changed; to the document it is all new.
        let mut engine = Engine::new();
        engine.load_html("<ul id=list></ul>");
        engine.resize(100, 50);
        engine.set_verifying(true);
        let list = engine.query(None, "#list").unwrap().unwrap();
        let item = engine.create_element("li").unwrap();
        let inner = engine.create_element("b").unwrap();
        engine.insert(item, inner, None).unwrap();
        assert!(engine.prepare().0.is_some());
        assert_eq!(engine.verify(), Ok(()));
        engine.insert(list, item, None).unwrap();
        engine.set_attr(item, "id", "yeni").unwrap();
        engine.set_attr(inner, "title", "iç").unwrap();
        assert!(engine.prepare().0.is_some());
        assert_eq!(engine.verify(), Ok(()));
        assert_eq!(
            engine.stats().changes,
            1,
            "the list's children, nothing else"
        );
    }

    #[test]
    fn no_frame_shows_a_transaction_halfway() {
        let mut engine = Engine::new();
        engine.load_html("<p id=p>bir</p>");
        engine.resize(100, 50);
        assert!(engine.prepare().0.is_some());
        let p = engine.query(None, "#p").unwrap().unwrap();
        engine.begin();
        engine.begin();
        engine.set_text(p, "iki").unwrap();
        assert!(engine.prepare().0.is_none(), "inside a transaction");
        engine.commit().unwrap();
        assert!(engine.prepare().0.is_none(), "the outer one is still open");
        engine.set_text(p, "üç").unwrap();
        engine.commit().unwrap();
        assert!(engine.prepare().0.is_some(), "all of it at once");
        // Both changes (each records the element and the text node it
        // makes), coalesced into one.
        assert_eq!(engine.stats().recorded, 4);
        assert_eq!(engine.stats().changes, 1);
        assert_eq!(engine.commit(), Err(Status::InvalidArgument));
    }

    #[test]
    fn the_oracle_says_where_a_frame_parts_from_it() {
        use crate::list::{DisplayItem, DisplayList};
        let rect = |x: f32| DisplayItem::Rect {
            x,
            y: 0.0,
            width: 10.0,
            height: 10.0,
            color: [0, 0, 0, 255],
        };
        let frame = DisplayList {
            canvas: [255; 4],
            items: vec![rect(0.0), rect(10.0)],
        };
        assert_eq!(compare(&frame, &frame.clone()), Ok(()));
        let moved = DisplayList {
            canvas: [255; 4],
            items: vec![rect(0.0), rect(11.0)],
        };
        let parted = compare(&frame, &moved).unwrap_err();
        assert!(parted.contains("item 1"), "{parted}");
        let shorter = DisplayList {
            canvas: [255; 4],
            items: vec![rect(0.0)],
        };
        let parted = compare(&frame, &shorter).unwrap_err();
        assert!(
            parted.contains("2 items") && parted.contains("1 items"),
            "{parted}"
        );
        let other_canvas = DisplayList {
            canvas: [0; 4],
            items: frame.items.clone(),
        };
        assert!(
            compare(&frame, &other_canvas)
                .unwrap_err()
                .contains("canvas")
        );
    }

    #[test]
    fn a_frame_counts_what_it_styled_laid_out_shaped_and_painted() {
        let mut engine = Engine::new();
        engine.load_html("<body><div><p>bir</p><p>iki</p></div><span>üç</span>");
        engine.resize(200, 100);
        assert_eq!(engine.stats(), FrameStats::default(), "no frame yet");
        let (prepared, _) = engine.prepare();
        let items = prepared.unwrap().list.items.len();
        // Styled: html, head, body, div, two p and the span. Boxes: the
        // document's (the initial containing block), html, body, div, the
        // two p, and the anonymous paragraph of the span's line. Shaped:
        // the two p and that anonymous paragraph.
        assert_eq!(
            engine.stats(),
            FrameStats {
                styled: 7,
                laid_out: 7,
                shaped: 3,
                items,
                // The first list: every item made from the document.
                listed: items,
                // A new document: all of it is new, nothing to coalesce.
                recorded: 0,
                changes: 0,
            }
        );
    }

    #[test]
    fn a_style_property_restyles_its_element_and_nothing_else() {
        let mut engine = Engine::new();
        let items: String = (0..100).map(|i| format!("<p id=p{i}>{i}</p>")).collect();
        engine.load_html(&format!(
            "<style>p {{ margin: 0 }}</style><div>{items}</div>"
        ));
        engine.resize(200, 100);
        engine.set_verifying(true);
        assert!(engine.prepare().0.is_some());
        // html, head, body, the div and the paragraphs.
        assert_eq!(engine.stats().styled, 104);
        let p = engine.query(None, "#p40").unwrap().unwrap();
        engine.set_style_property(p, "color", "red").unwrap();
        assert_eq!(
            engine.style_property(p, "color").unwrap().as_deref(),
            Some("red")
        );
        assert!(engine.prepare().0.is_some());
        assert_eq!(engine.verify(), Ok(()));
        assert_eq!(engine.stats().styled, 1);
        assert_eq!(
            engine.set_style_property(p, "color", "1px"),
            Err(Status::InvalidArgument)
        );
        engine.remove_style_property(p, "color").unwrap();
        assert!(engine.prepare().0.is_some());
        assert_eq!(engine.verify(), Ok(()));
        assert_eq!(engine.stats().styled, 1);
    }

    /// An engine showing `html`, verifying, with its first frame made.
    fn verifying(html: &str) -> Engine {
        let mut engine = Engine::new();
        engine.load_html(html);
        engine.resize(300, 200);
        engine.set_verifying(true);
        assert!(engine.prepare().0.is_some());
        engine
    }

    /// Make a frame, check it against the oracle, and say whether it laid
    /// anything out.
    fn laid_out(engine: &mut Engine) -> bool {
        assert!(engine.prepare().0.is_some());
        assert_eq!(engine.verify(), Ok(()));
        let stats = engine.stats();
        // Nothing laid out, nothing shaped.
        assert!(stats.laid_out > 0 || stats.shaped == 0, "{stats:?}");
        stats.laid_out > 0
    }

    #[test]
    fn a_paragraph_is_shaped_again_only_when_its_content_changes() {
        let items: String = (0..50)
            .map(|i| format!("<p id=p{i}>satır {i} <b>kalın</b></p>"))
            .collect();
        let mut engine = verifying(&format!("<div id=d>{items}</div>"));
        // Every paragraph, the first time.
        assert_eq!(engine.stats().shaped, 50);
        let (d, p) = (
            engine.query(None, "#d").unwrap().unwrap(),
            engine.query(None, "#p7").unwrap().unwrap(),
        );
        // Another width: the lines are broken again, the text is not
        // shaped again.
        engine.set_style_property(d, "width", "60px").unwrap();
        assert!(laid_out(&mut engine));
        assert_eq!(engine.stats().shaped, 0);
        // One paragraph's text: that paragraph.
        engine.set_text(p, "yeni").unwrap();
        assert!(laid_out(&mut engine));
        assert_eq!(engine.stats().shaped, 1);
        // A font size: the paragraph that has it.
        engine.set_style_property(p, "font-size", "30px").unwrap();
        assert!(laid_out(&mut engine));
        assert_eq!(engine.stats().shaped, 1);
        // An atomic inline's width goes into the shaping too.
        let atom = engine.create_element("span").unwrap();
        engine
            .set_attr(atom, "style", "display: inline-block; width: 10px")
            .unwrap();
        engine.insert(p, atom, None).unwrap();
        assert!(laid_out(&mut engine));
        engine.set_style_property(atom, "width", "40px").unwrap();
        assert!(laid_out(&mut engine));
        assert_eq!(engine.stats().shaped, 1);
    }

    /// How many boxes the last frame laid out, checked against the oracle.
    fn boxes_laid_out(engine: &mut Engine) -> usize {
        assert!(engine.prepare().0.is_some());
        assert_eq!(engine.verify(), Ok(()));
        engine.stats().laid_out
    }

    #[test]
    fn only_the_boxes_a_change_reaches_are_laid_out_again() {
        let items: String = (0..100)
            .map(|i| format!("<p id=p{i}>satır {i}</p>"))
            .collect();
        let mut engine = verifying(&format!("<div id=d>{items}</div>"));
        let find = |engine: &Engine, selector| engine.query(None, selector).unwrap().unwrap();
        let (d, p) = (find(&engine, "#d"), find(&engine, "#p7"));
        // The paragraph and the boxes that hold it: the document's, html,
        // body, the div.
        engine.set_text(p, "yeni ve daha uzun bir satır").unwrap();
        assert_eq!(boxes_laid_out(&mut engine), 5);
        engine.set_style_property(p, "padding", "4px").unwrap();
        assert_eq!(boxes_laid_out(&mut engine), 5);
        // A new paragraph: it and its ancestors.
        let made = engine.create_element("p").unwrap();
        engine.set_text(made, "eklendi").unwrap();
        engine.insert(d, made, Some(p)).unwrap();
        assert_eq!(boxes_laid_out(&mut engine), 5);
        engine.remove(made).unwrap();
        assert_eq!(boxes_laid_out(&mut engine), 4);
        // The last one: what is left is what was before it.
        let last = find(&engine, "#p99");
        engine.remove(last).unwrap();
        assert_eq!(boxes_laid_out(&mut engine), 4);
        // Another width for the div: what is in it is laid out again.
        engine.set_style_property(d, "width", "80px").unwrap();
        assert!(boxes_laid_out(&mut engine) > 100);
    }

    #[test]
    fn a_containing_block_built_again_gets_back_what_was_taken_as_it_was() {
        // The block that finds the absolutely positioned element is taken
        // from the last tree as it is; its containing block is built again
        // and must get the element back.
        let mut engine = verifying(
            "<div style='position: relative; width: 200px'>\
             <div><span style='position: absolute; left: 30px; top: 5px'>mutlak</span>iç</div>\
             <p id=t>metin</p></div>",
        );
        let t = engine.query(None, "#t").unwrap().unwrap();
        for text in ["daha uzun bir metin", "kısa", ""] {
            engine.set_text(t, text).unwrap();
            boxes_laid_out(&mut engine);
        }
    }

    #[test]
    fn what_moves_is_built_where_it_went_with_what_it_holds() {
        // The positioned element inside goes with it to another containing
        // block, though its own style does not change.
        let mut engine = verifying(
            "<div id=a style='position: relative; margin-left: 10px'>\
             <div id=m><b style='position: absolute; left: 5px'>x</b>m</div></div>\
             <div id=b style='position: relative; margin-left: 70px'></div>",
        );
        let find = |engine: &Engine, selector| engine.query(None, selector).unwrap().unwrap();
        let (a, b, m) = (
            find(&engine, "#a"),
            find(&engine, "#b"),
            find(&engine, "#m"),
        );
        engine.insert(b, m, None).unwrap();
        boxes_laid_out(&mut engine);
        engine.insert(a, m, None).unwrap();
        boxes_laid_out(&mut engine);
    }

    #[test]
    fn an_ancestor_that_changes_reaches_what_it_holds() {
        let mut engine = verifying(
            "<div id=d><p>başlık <span>İSTANBUL</span></p><div>iç <b>blok</b></div></div>",
        );
        let d = engine.query(None, "#d").unwrap().unwrap();
        // The language picks the case mapping of every text inside.
        engine.set_attr(d, "lang", "tr").unwrap();
        engine
            .set_style_property(d, "text-transform", "lowercase")
            .unwrap();
        boxes_laid_out(&mut engine);
        engine.set_style_property(d, "display", "none").unwrap();
        boxes_laid_out(&mut engine);
        engine.remove_style_property(d, "display").unwrap();
        boxes_laid_out(&mut engine);
    }

    #[test]
    fn the_arena_growing_moves_the_anonymous_boxes_up() {
        // Anonymous paragraphs and placeholders come after the element
        // boxes; an arena that outgrows the room kept moves them, and every
        // index that names one: an atom's parent, a placeholder.
        let mut engine = verifying(
            "<div id=d>önce <i id=atom style='display: inline-block; width: 10px'>a</i>\
             <b style='position: absolute; top: 2px'>çapa</b>\
             <p>blok</p><div style='position: absolute; left: 3px'>yer</div><p>ikinci</p>\
             sonra</div><ul id=list></ul>",
        );
        let find = |engine: &Engine, selector| engine.query(None, selector).unwrap().unwrap();
        let (list, atom) = (find(&engine, "#list"), find(&engine, "#atom"));
        for round in 0..3 {
            for i in 0..150 {
                let item = engine.create_element("li").unwrap();
                engine.set_text(item, &format!("{round}.{i}")).unwrap();
                engine.insert(list, item, None).unwrap();
            }
            boxes_laid_out(&mut engine);
            // What the moved boxes name must still be them.
            engine
                .set_style_property(atom, "width", &format!("{}px", 20 + round * 15))
                .unwrap();
            boxes_laid_out(&mut engine);
        }
    }

    #[test]
    fn a_containing_block_that_changes_takes_what_is_deep_inside() {
        // Only the ancestor's style changes; the positioned element deep
        // inside goes to it all the same.
        let mut engine = verifying(
            "<div id=k style='margin-left: 40px'><div><p>iç <span style='position: absolute; left: 7px'>m</span></p></div></div>",
        );
        let k = engine.query(None, "#k").unwrap().unwrap();
        engine
            .set_style_property(k, "position", "relative")
            .unwrap();
        boxes_laid_out(&mut engine);
        engine.remove_style_property(k, "position").unwrap();
        boxes_laid_out(&mut engine);
    }

    #[test]
    fn a_slot_given_to_another_element_is_built_for_it() {
        let mut engine = verifying("<div id=d><p id=a style='height: 30px'></p><p>sonra</p></div>");
        let find = |engine: &Engine, selector| engine.query(None, selector).unwrap().unwrap();
        let (d, a) = (find(&engine, "#d"), find(&engine, "#a"));
        engine.remove(a).unwrap();
        // The arena gives the new element the removed one's slot.
        let made = engine.create_element("h1").unwrap();
        assert_eq!(
            NodeId::from_bits(made).unwrap().index(),
            NodeId::from_bits(a).unwrap().index()
        );
        engine.insert(d, made, None).unwrap();
        boxes_laid_out(&mut engine);
    }

    #[test]
    fn a_colour_that_changed_without_layout_is_in_the_next_layout() {
        // A colour that only repaints leaves the tree as it was; when a
        // later frame lays the paragraph out again, the tree must have it.
        let mut engine = verifying(
            "<p id=a style='width: 100px'>a</p><p>bir <span id=s style='color: red'>iki</span> üç</p>",
        );
        let find = |engine: &Engine, selector| engine.query(None, selector).unwrap().unwrap();
        let (a, s) = (find(&engine, "#a"), find(&engine, "#s"));
        engine.set_style_property(s, "color", "blue").unwrap();
        assert!(!laid_out(&mut engine));
        // The paragraph above grows: this one moves down.
        engine
            .set_text(a, "uzun bir metin ki birkaç satıra sarılır")
            .unwrap();
        boxes_laid_out(&mut engine);
    }

    #[test]
    fn what_changes_nothing_taffy_reads_lays_nothing_out() {
        // A caret blinking: `visibility` runs layout, but no box's inputs
        // change, the document's box included, which holds the viewport.
        let mut engine = verifying(
            "<p>a</p><span id=c style='display: inline-block; width: 1px; height: 16px'></span>",
        );
        let c = engine.query(None, "#c").unwrap().unwrap();
        for visibility in ["hidden", "visible"] {
            engine
                .set_style_property(c, "visibility", visibility)
                .unwrap();
            assert_eq!(boxes_laid_out(&mut engine), 0);
        }
        // Only the viewport changes.
        engine.resize(250, 200);
        assert!(boxes_laid_out(&mut engine) > 0);
    }

    /// The pointer moved onto the middle of `node`'s box.
    fn hover(engine: &mut Engine, node: u64) {
        let model = engine.node_box(node).unwrap().unwrap();
        engine.pointer(&PointerInput {
            kind: PointerKind::Move,
            x: model.x + model.width / 2.0,
            y: model.y + model.height / 2.0,
            button: PointerButton::None,
            modifiers: Modifiers::default(),
        });
    }

    #[test]
    fn a_style_the_pointer_changes_splits_the_text_again() {
        // Nothing in the document changes, but the hover makes the span's
        // colour its neighbours': the runs split elsewhere, and the
        // paragraph is laid out again.
        let mut engine = verifying(
            "<style>p { color: #000 } #s { color: #c00 } p:hover #s { color: #000 }</style>\
             <p id=p>bir <span id=s>iki</span> üç</p>",
        );
        let p = engine.query(None, "#p").unwrap().unwrap();
        hover(&mut engine, p);
        assert!(laid_out(&mut engine));
    }

    #[test]
    fn fonts_the_host_offers_later_are_asked_for() {
        // Nothing in the document changes, but the host's catalogue comes
        // after the first frame and has the family the page names.
        let mut engine = verifying("<p style='font-family: Erk Test'>bir</p>");
        engine.set_fonts(crate::FontCatalog {
            families: vec!["Erk Test".to_owned()],
            generic: Vec::new(),
            fallback: Vec::new(),
        });
        let (_, requests) = engine.prepare();
        assert!(
            requests
                .iter()
                .any(|request| request.url.starts_with("font:Erk Test")),
            "{requests:?}"
        );
    }

    #[test]
    fn a_url_the_pointer_names_is_asked_for() {
        // Nothing in the document changes, but the hover names an image.
        let mut engine =
            verifying("<style>p:hover { background-image: url(hover.png) }</style><p id=p>bir</p>");
        let p = engine.query(None, "#p").unwrap().unwrap();
        hover(&mut engine, p);
        let (_, requests) = engine.prepare();
        assert!(
            requests
                .iter()
                .any(|request| request.url.ends_with("hover.png")),
            "{requests:?}"
        );
    }

    #[test]
    fn a_scroll_inside_the_band_moves_the_last_list() {
        // The viewport is 200 px high: a band is two cells of 200 px.
        let lines: String = (0..60).map(|i| format!("<p>satır {i}</p>")).collect();
        let mut engine = verifying(&format!("<body style='margin: 0'>{lines}"));
        let scroll = |engine: &mut Engine, dy: f32| {
            engine.wheel((0.0, dy), (10.0, 10.0));
            assert!(engine.prepare().0.is_some());
            assert_eq!(engine.verify(), Ok(()));
            engine.stats().listed
        };
        // Inside the first cell, then into the next, then back. A box is
        // where it is painted.
        let p = engine.query(None, "p").unwrap().unwrap();
        let top = |engine: &Engine| engine.node_box(p).unwrap().unwrap().y;
        let before = top(&engine);
        assert_eq!(scroll(&mut engine, 50.0), 0);
        assert_eq!(top(&engine), before - 50.0);
        assert_eq!(scroll(&mut engine, 120.5), 0);
        assert!(scroll(&mut engine, 100.0) > 0);
        assert_eq!(scroll(&mut engine, 30.0), 0);
        assert!(scroll(&mut engine, -200.0) > 0);
    }

    #[test]
    fn a_page_that_grows_to_scroll_under_the_pointer_shows_its_scroll_bar() {
        // The page grows long enough to scroll in the frame the pointer
        // moves over it: that frame shows the document's scroll bar.
        let mut engine = verifying("<p id=p style='margin: 0'>kısa</p>");
        let p = engine.query(None, "#p").unwrap().unwrap();
        engine.set_style_property(p, "height", "1000px").unwrap();
        engine.pointer(&PointerInput {
            kind: PointerKind::Move,
            x: 10.0,
            y: 10.0,
            button: PointerButton::None,
            modifiers: Modifiers::default(),
        });
        let (prepared, _) = engine.prepare();
        assert_eq!(engine.verify(), Ok(()));
        let thumb = prepared.unwrap().list.items.into_iter().any(|item| {
            matches!(item, crate::list::DisplayItem::RoundedRect { color, .. } if color == [0, 0, 0, 102])
        });
        assert!(thumb, "no scroll bar");
    }

    #[test]
    fn the_pointer_moving_into_a_scroll_container_shows_its_scroll_bar() {
        // Nothing but the pointer changes: the last frame's list would do,
        // but the container's own scroll bar is in it.
        let mut engine = verifying(
            "<div style='height: 50px; overflow: auto'><p style='height: 500px'>uzun</p></div>",
        );
        engine.pointer(&PointerInput {
            kind: PointerKind::Move,
            x: 10.0,
            y: 20.0,
            button: PointerButton::None,
            modifiers: Modifiers::default(),
        });
        let (prepared, _) = engine.prepare();
        assert_eq!(engine.verify(), Ok(()));
        let thumb = prepared.unwrap().list.items.into_iter().any(|item| {
            matches!(item, crate::list::DisplayItem::RoundedRect { color, .. } if color == [0, 0, 0, 102])
        });
        assert!(thumb, "no scroll bar");
    }

    #[test]
    fn a_page_with_only_focus_visible_rules_shows_the_keyboards_focus() {
        // No `:focus` rule: the focus still matters to how the page looks.
        let mut engine = verifying(
            "<style>:focus-visible { color: rgb(255, 0, 0) }</style><button id=b>b</button>",
        );
        let b = engine.query(None, "#b").unwrap().unwrap();
        let tab = KeyInput {
            key: crate::messages::Key::Tab,
            state: crate::messages::KeyState::Down,
            modifiers: Modifiers::default(),
        };
        engine.key(&tab);
        engine.key_default(&tab);
        assert!(engine.prepare().0.is_some(), "the focus shows: a frame");
        assert_eq!(engine.verify(), Ok(()));
        let style = engine.computed_style(b).unwrap().unwrap();
        assert!(style.contains("color: rgb(255, 0, 0)"), "{style}");
    }

    #[test]
    fn a_checkboxs_value_set_is_its_attribute_and_restyles_it() {
        // A checkbox's value is its `value` attribute, which selectors see.
        let mut engine = verifying(
            "<style>input { display: block; width: 20px; height: 20px }\
             [value=a] { background: rgb(255, 0, 0) }</style><input type=checkbox id=k value=b>",
        );
        let k = engine.query(None, "#k").unwrap().unwrap();
        engine.set_value(k, "a").unwrap();
        assert!(engine.prepare().0.is_some(), "the value shows: a frame");
        assert_eq!(engine.verify(), Ok(()));
        let style = engine.computed_style(k).unwrap().unwrap();
        assert!(
            style.contains("background-color: rgb(255, 0, 0)"),
            "{style}"
        );
    }

    /// The text the frame paints, run by run.
    fn painted_text(prepared: &Prepared) -> String {
        prepared
            .list
            .items
            .iter()
            .filter_map(|item| match item {
                crate::list::DisplayItem::Glyphs(run) => Some(run.text.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_text_field_shows_its_value_and_a_value_set_is_laid_out_again() {
        let mut engine = verifying(
            "<p>ad <input id=t value=eski> son</p><textarea id=a>bir iki</textarea>\
             <input id=s type=password value=gizli><input type=checkbox value=evet>",
        );
        let (t, a, s) = (
            engine.query(None, "#t").unwrap().unwrap(),
            engine.query(None, "#a").unwrap().unwrap(),
            engine.query(None, "#s").unwrap().unwrap(),
        );
        engine.set_value(t, "yeni").unwrap();
        engine.set_value(a, "üç").unwrap();
        let (prepared, _) = engine.prepare();
        assert_eq!(engine.verify(), Ok(()));
        let text = painted_text(&prepared.unwrap());
        assert!(text.contains("yeni") && !text.contains("eski"), "{text}");
        assert!(text.contains("üç") && !text.contains("bir"), "{text}");
        // A password shows as many bullets as it has characters.
        assert!(text.contains("•••••") && !text.contains("gizli"), "{text}");
        // A checkbox is no text field: its value is not shown.
        assert!(!text.contains("evet"), "{text}");
        // Only the two fields are shaped again: the paragraph around the
        // input keeps its text, only measured again with the field.
        assert_eq!(engine.stats().shaped, 2, "{:?}", engine.stats());
        engine.set_value(s, "").unwrap();
        assert!(engine.prepare().0.is_some());
        assert_eq!(engine.verify(), Ok(()));
        // A field made a password hides what it shows.
        engine.set_attr(t, "type", "password").unwrap();
        let (prepared, _) = engine.prepare();
        assert_eq!(engine.verify(), Ok(()));
        let text = painted_text(&prepared.unwrap());
        assert!(text.contains("••••") && !text.contains("yeni"), "{text}");
    }

    #[test]
    fn what_a_field_shows_is_the_field_to_a_pointer() {
        // Its text is no node's: a point on it finds the field.
        let engine = verifying(
            "<body style='margin: 0'><input id=t value=metin style='font: 20px sans-serif; border: 0; padding: 0'>",
        );
        let t = engine.query(None, "#t").unwrap().unwrap();
        assert_eq!(engine.inspect_at(10.0, 10.0), Some(t));
    }

    #[test]
    fn a_field_paints_its_value_in_its_own_colour() {
        // Not in the colour of the element around it.
        let mut engine = verifying(
            "<p style='color: rgb(0, 0, 0)'><input id=t value=eski style='color: rgb(255, 0, 0)'></p>",
        );
        let t = engine.query(None, "#t").unwrap().unwrap();
        engine.set_value(t, "kırmızı").unwrap();
        let (prepared, _) = engine.prepare();
        assert_eq!(engine.verify(), Ok(()));
        let colours: Vec<[u8; 4]> = prepared
            .unwrap()
            .list
            .items
            .iter()
            .filter_map(|item| match item {
                crate::list::DisplayItem::Glyphs(run) => Some(run.color),
                _ => None,
            })
            .collect();
        assert_eq!(colours, [[255, 0, 0, 255]]);
    }

    #[test]
    fn a_fields_value_keeps_its_spaces_whatever_its_white_space() {
        // `white-space: normal` wraps a textarea's value but keeps its
        // spaces and line breaks (WPT textarea-always-preserves-spaces).
        let mut engine =
            verifying("<textarea id=a style='white-space: normal; width: 300px'>eski</textarea>");
        let a = engine.query(None, "#a").unwrap().unwrap();
        engine.set_value(a, "a    b\nc").unwrap();
        let (prepared, _) = engine.prepare();
        assert_eq!(engine.verify(), Ok(()));
        let prepared = prepared.unwrap();
        let text = painted_text(&prepared);
        assert!(text.contains("a    b"), "{text:?}");
        // The line break kept: "c" on the line below. (A textarea is as
        // high as its `rows`, not as what it holds.)
        let top = |wanted: &str| {
            prepared
                .list
                .items
                .iter()
                .find_map(|item| match item {
                    crate::list::DisplayItem::Glyphs(run) if run.text.contains(wanted) => {
                        run.glyphs.first().map(|glyph| glyph.y)
                    }
                    _ => None,
                })
                .expect("painted")
        };
        assert!(top("c") > top("a    b") + 5.0);
    }

    #[test]
    fn a_text_field_laid_out_inline_still_shows_its_value() {
        // What it shows is not its children's: an inline field is atomic.
        let mut engine = verifying(
            "<p>a <input id=t value=değer style='display: inline'> b <span><input value=iç style='display: inline'></span></p>",
        );
        let (prepared, _) = engine.prepare();
        assert!(prepared.is_none(), "nothing changed since the first frame");
        let t = engine.query(None, "#t").unwrap().unwrap();
        engine.set_value(t, "yeni").unwrap();
        let (prepared, _) = engine.prepare();
        assert_eq!(engine.verify(), Ok(()));
        let text = painted_text(&prepared.unwrap());
        assert!(text.contains("yeni") && text.contains("iç"), "{text}");
    }

    #[test]
    fn a_fields_value_attribute_shows_until_a_value_is_set() {
        let mut engine = verifying("<input id=t value=bir>");
        let t = engine.query(None, "#t").unwrap().unwrap();
        engine.set_attr(t, "value", "iki").unwrap();
        let (prepared, _) = engine.prepare();
        assert_eq!(engine.verify(), Ok(()));
        assert!(painted_text(&prepared.unwrap()).contains("iki"));
        engine.set_value(t, "üç").unwrap();
        assert!(engine.prepare().0.is_some());
        engine.set_attr(t, "value", "dört").unwrap();
        let (prepared, _) = engine.prepare();
        assert_eq!(engine.verify(), Ok(()));
        let text = painted_text(&prepared.unwrap());
        assert!(text.contains("üç") && !text.contains("dört"), "{text}");
    }

    #[test]
    fn a_text_field_sits_on_the_line_of_the_text_around_it() {
        // Its text's baseline is the line's, though it clips what overflows.
        let engine = verifying(
            "<p id=p style='font: 16px sans-serif; margin: 0'>ad <input id=t value=x style='font: 16px sans-serif; border: 0; padding: 0'></p>",
        );
        let p = engine.query(None, "#p").unwrap().unwrap();
        let t = engine.query(None, "#t").unwrap().unwrap();
        let (line, field) = (
            engine.node_box(p).unwrap().unwrap(),
            engine.node_box(t).unwrap().unwrap(),
        );
        // One line: the field's box is the line's height, at its top.
        assert!((field.y - line.y).abs() < 1.0, "{field:?} in {line:?}");
        assert!(field.height <= line.height + 0.5, "{field:?} in {line:?}");
    }

    /// Press `key` (down and up) as a host does that prevents nothing: its
    /// events, then what it does by default.
    fn press(engine: &mut Engine, key: crate::messages::Key) -> Vec<EventKind> {
        let mut kinds = Vec::new();
        for state in [
            crate::messages::KeyState::Down,
            crate::messages::KeyState::Up,
        ] {
            let input = KeyInput {
                key: key.clone(),
                state,
                modifiers: Modifiers::default(),
            };
            kinds.extend(engine.key(&input).map(|event| event.kind));
            kinds.extend(
                engine
                    .key_default(&input)
                    .into_iter()
                    .map(|event| event.kind),
            );
        }
        kinds
    }

    fn type_text(engine: &mut Engine, text: &str) {
        for c in text.chars() {
            press(engine, crate::messages::Key::Character(c.to_string()));
        }
    }

    /// The caret the frame paints: a rectangle one pixel wide.
    fn caret(prepared: &Prepared) -> Option<(f32, f32)> {
        prepared.list.items.iter().find_map(|item| match item {
            crate::list::DisplayItem::Rect { x, y, width, .. } if *width == 1.0 => Some((*x, *y)),
            _ => None,
        })
    }

    #[test]
    fn typing_in_a_focused_field_edits_its_value_at_the_caret() {
        use crate::messages::Key;
        let mut engine = verifying("<input id=t value=ab><button id=b>b</button>");
        let t = engine.query(None, "#t").unwrap().unwrap();
        // Tab focuses the field: its caret is at the end.
        press(&mut engine, Key::Tab);
        let (prepared, _) = engine.prepare();
        assert_eq!(engine.verify(), Ok(()));
        let at_end = caret(&prepared.unwrap()).expect("a caret");
        // Each character typed is an input event and a frame.
        let kinds = press(&mut engine, Key::Character("ç".to_owned()));
        assert_eq!(
            kinds,
            [EventKind::KeyDown, EventKind::Input, EventKind::KeyUp]
        );
        assert_eq!(engine.value(t), Ok(Some("abç".to_owned())));
        let (prepared, _) = engine.prepare();
        assert_eq!(engine.verify(), Ok(()));
        let moved = caret(&prepared.unwrap()).expect("a caret");
        assert!(moved.0 > at_end.0, "{moved:?} after {at_end:?}");
        // The caret moves without editing: a frame, no input event.
        for key in [Key::ArrowLeft, Key::ArrowLeft] {
            assert_eq!(
                press(&mut engine, key),
                [EventKind::KeyDown, EventKind::KeyUp]
            );
        }
        let (prepared, _) = engine.prepare();
        assert_eq!(engine.verify(), Ok(()));
        let back = caret(&prepared.unwrap()).expect("a caret");
        assert!(back.0 < moved.0, "{back:?} before {moved:?}");
        press(&mut engine, Key::Space);
        type_text(&mut engine, "x");
        assert_eq!(engine.value(t), Ok(Some("a xbç".to_owned())));
        press(&mut engine, Key::Backspace);
        press(&mut engine, Key::Delete);
        assert_eq!(engine.value(t), Ok(Some("a ç".to_owned())));
        press(&mut engine, Key::Home);
        type_text(&mut engine, "[");
        press(&mut engine, Key::End);
        type_text(&mut engine, "]");
        assert_eq!(engine.value(t), Ok(Some("[a ç]".to_owned())));
        press(&mut engine, Key::Home);
        press(&mut engine, Key::ArrowRight);
        type_text(&mut engine, "-");
        press(&mut engine, Key::End);
        assert_eq!(engine.value(t), Ok(Some("[-a ç]".to_owned())));
        // Enter types nothing into one line, and is no input event; up and
        // down move nowhere.
        assert_eq!(
            press(&mut engine, Key::Enter),
            [EventKind::KeyDown, EventKind::KeyUp]
        );
        press(&mut engine, Key::ArrowUp);
        type_text(&mut engine, "!");
        assert_eq!(engine.value(t), Ok(Some("[-a ç]!".to_owned())));
        // Leaving the field: the caret goes with the focus.
        press(&mut engine, Key::Tab);
        let (prepared, _) = engine.prepare();
        assert_eq!(engine.verify(), Ok(()));
        assert_eq!(caret(&prepared.unwrap()), None);
    }

    #[test]
    fn the_caret_steps_over_a_grapheme_cluster_whole() {
        use crate::messages::Key;
        // "e" with a combining acute accent, and a flag of two regional
        // indicators: one step each, one deletion each.
        let mut engine = verifying("<input id=t value='ae\u{301}🇹🇷b'>");
        let t = engine.query(None, "#t").unwrap().unwrap();
        press(&mut engine, Key::Tab);
        press(&mut engine, Key::ArrowLeft);
        press(&mut engine, Key::Backspace);
        assert_eq!(engine.value(t), Ok(Some("ae\u{301}b".to_owned())));
        press(&mut engine, Key::ArrowLeft);
        press(&mut engine, Key::Delete);
        assert_eq!(engine.value(t), Ok(Some("ab".to_owned())));
    }

    #[test]
    fn a_textarea_takes_line_breaks_and_moves_between_its_lines() {
        use crate::messages::Key;
        let mut engine = verifying("<textarea id=a>bir\nikinci</textarea>");
        let a = engine.query(None, "#a").unwrap().unwrap();
        press(&mut engine, Key::Tab);
        // From the end of "ikinci", up: the first line is shorter.
        press(&mut engine, Key::ArrowUp);
        type_text(&mut engine, "!");
        assert_eq!(engine.value(a), Ok(Some("bir!\nikinci".to_owned())));
        // Down from the fourth column of the first line: after "ikin".
        press(&mut engine, Key::ArrowDown);
        press(&mut engine, Key::Enter);
        assert_eq!(engine.value(a), Ok(Some("bir!\nikin\nci".to_owned())));
        press(&mut engine, Key::Home);
        type_text(&mut engine, ">");
        assert_eq!(engine.value(a), Ok(Some("bir!\nikin\n>ci".to_owned())));
        // End: the end of the line the caret is on, not of the text.
        press(&mut engine, Key::ArrowUp);
        press(&mut engine, Key::End);
        type_text(&mut engine, "<");
        assert_eq!(engine.value(a), Ok(Some("bir!\nikin<\n>ci".to_owned())));
        let (prepared, _) = engine.prepare();
        assert_eq!(engine.verify(), Ok(()));
        assert!(caret(&prepared.unwrap()).is_some());
    }

    #[test]
    fn a_field_clicked_into_shows_its_caret_after_its_value() {
        // Focused by a press past its value, the field shows its caret at
        // its end: a password's after its last bullet.
        let mut engine = verifying(
            "<body style='margin: 0'><input id=t type=password value=abc style='font: 20px sans-serif; border: 0; padding: 0'>",
        );
        for kind in [PointerKind::Down, PointerKind::Up] {
            engine.pointer(&PointerInput {
                kind,
                x: 150.0,
                y: 5.0,
                button: PointerButton::Primary,
                modifiers: Modifiers::default(),
            });
        }
        let (prepared, _) = engine.prepare();
        assert_eq!(engine.verify(), Ok(()));
        let prepared = prepared.expect("the caret shows: a frame");
        let (x, _) = caret(&prepared).expect("a caret");
        let right = prepared
            .list
            .items
            .iter()
            .filter_map(|item| match item {
                crate::list::DisplayItem::Glyphs(run) => {
                    run.glyphs.iter().map(|glyph| glyph.x).reduce(f32::max)
                }
                _ => None,
            })
            .reduce(f32::max)
            .expect("bullets");
        assert!(x > right, "the caret at {x}, the last bullet at {right}");
    }

    #[test]
    fn the_caret_blinks_on_the_hosts_clock_and_shows_while_typing() {
        use crate::messages::Key;
        const MS: u64 = 1_000_000;
        let mut engine = verifying("<input id=t value=a><p>metin</p><button>b</button>");
        // No field focused: nothing moves by itself.
        assert_eq!(engine.next_tick(), None);
        engine.set_time(100 * MS);
        assert!(engine.prepare().0.is_none(), "no caret, no frame");
        press(&mut engine, Key::Tab);
        let (prepared, _) = engine.prepare();
        assert!(caret(&prepared.unwrap()).is_some(), "shows at once");
        // Half a second after it last moved, the caret hides.
        assert_eq!(engine.next_tick(), Some(600 * MS));
        engine.set_time(599 * MS);
        assert!(engine.prepare().0.is_none(), "not yet");
        engine.set_time(600 * MS);
        let (prepared, _) = engine.prepare();
        assert_eq!(engine.verify(), Ok(()));
        assert_eq!(caret(&prepared.unwrap()), None, "hidden");
        assert_eq!(engine.next_tick(), Some(1100 * MS));
        // Typing shows it at once, and the blink starts again from there.
        engine.set_time(800 * MS);
        type_text(&mut engine, "b");
        let (prepared, _) = engine.prepare();
        assert_eq!(engine.verify(), Ok(()));
        assert!(caret(&prepared.unwrap()).is_some(), "shows while typing");
        assert_eq!(engine.next_tick(), Some(1300 * MS));
        engine.set_time(1300 * MS);
        assert_eq!(caret(&engine.prepare().0.unwrap()), None);
        // A clock that goes back is taken as standing still.
        engine.set_time(200 * MS);
        assert_eq!(engine.next_tick(), Some(1800 * MS));
        // Leaving the field stops the blink.
        press(&mut engine, Key::Tab);
        assert_eq!(engine.next_tick(), None);
        assert!(engine.prepare().0.is_some(), "the caret goes");
        engine.set_time(1900 * MS);
        assert!(engine.prepare().0.is_none(), "no caret, no blink");
    }

    fn click(engine: &mut Engine, (x, y): (f32, f32)) {
        for kind in [PointerKind::Down, PointerKind::Up] {
            engine.pointer(&PointerInput {
                kind,
                x,
                y,
                button: PointerButton::Primary,
                modifiers: Modifiers::default(),
            });
        }
    }

    /// The painted glyphs' left and right edges.
    fn glyph_span(prepared: &Prepared) -> (f32, f32) {
        let xs: Vec<f32> = prepared
            .list
            .items
            .iter()
            .filter_map(|item| match item {
                crate::list::DisplayItem::Glyphs(run) => {
                    Some(run.glyphs.iter().map(|glyph| glyph.x))
                }
                _ => None,
            })
            .flatten()
            .collect();
        (
            xs.iter().copied().fold(f32::INFINITY, f32::min),
            xs.iter().copied().fold(f32::NEG_INFINITY, f32::max),
        )
    }

    #[test]
    fn a_press_puts_the_caret_where_it_pressed() {
        use crate::messages::Key;
        const FIELD: &str = "style='font: 20px sans-serif; border: 0; padding: 0; width: 300px'";
        let mut engine = verifying(&format!(
            "<body style='margin: 0'><input id=t value=abcdef {FIELD}><br>\
             <input id=s type=password value=gizli {FIELD}>"
        ));
        let (t, s) = (
            engine.query(None, "#t").unwrap().unwrap(),
            engine.query(None, "#s").unwrap().unwrap(),
        );
        // Where the third letter ends: a third of the way along "abcdef"
        // is near enough for even-width letters.
        engine.set_value(t, "abcdef").unwrap();
        let (prepared, _) = engine.prepare();
        let xs: Vec<f32> = prepared
            .unwrap()
            .list
            .items
            .iter()
            .filter_map(|item| match item {
                crate::list::DisplayItem::Glyphs(run) if run.text == "abcdef" => {
                    Some(run.glyphs.iter().map(|glyph| glyph.x).collect::<Vec<_>>())
                }
                _ => None,
            })
            .flatten()
            .collect();
        // Just before the fourth letter: after the third.
        click(&mut engine, (xs[3] - 1.0, 10.0));
        type_text(&mut engine, "X");
        assert_eq!(engine.value(t), Ok(Some("abcXdef".to_owned())));
        // Pressed again further on, in the same field.
        click(&mut engine, (2.0, 10.0));
        type_text(&mut engine, "<");
        assert_eq!(engine.value(t), Ok(Some("<abcXdef".to_owned())));
        // A password counts its bullets: pressed past the end, at the end;
        // pressed after the second bullet, after the second letter.
        let line = engine.node_box(s).unwrap().unwrap().y + 10.0;
        click(&mut engine, (290.0, line));
        press(&mut engine, Key::Backspace);
        assert_eq!(engine.value(s), Ok(Some("gizl".to_owned())));
        let (prepared, _) = engine.prepare();
        let bullets: Vec<f32> = prepared
            .unwrap()
            .list
            .items
            .iter()
            .filter_map(|item| match item {
                crate::list::DisplayItem::Glyphs(run) if run.text.starts_with('•') => {
                    Some(run.glyphs.iter().map(|glyph| glyph.x).collect::<Vec<_>>())
                }
                _ => None,
            })
            .flatten()
            .collect();
        click(&mut engine, (bullets[2] - 1.0, line));
        press(&mut engine, Key::Backspace);
        assert_eq!(engine.value(s), Ok(Some("gzl".to_owned())));
        let (prepared, _) = engine.prepare();
        assert_eq!(engine.verify(), Ok(()));
        assert!(caret(&prepared.unwrap()).is_some());
    }

    #[test]
    fn a_caret_out_of_view_scrolls_its_field_to_show_it() {
        use crate::messages::Key;
        let mut engine = verifying(
            "<body style='margin: 0'><input id=t value='bu değer dar alana hiç sığmayacak kadar uzun' \
             style='font: 16px sans-serif; border: 0; padding: 0; width: 60px'><button>b</button>",
        );
        // Focused, the caret is at the end: the field shows the end.
        press(&mut engine, Key::Tab);
        let (prepared, _) = engine.prepare();
        assert_eq!(engine.verify(), Ok(()));
        let prepared = prepared.unwrap();
        let (x, _) = caret(&prepared).expect("a caret");
        assert!((0.0..60.0).contains(&x), "the caret at {x}, in view");
        let (left, _) = glyph_span(&prepared);
        assert!(left < -100.0, "the start scrolled out, at {left}");
        // Home: back to the start.
        press(&mut engine, Key::Home);
        let (prepared, _) = engine.prepare();
        assert_eq!(engine.verify(), Ok(()));
        let (left, _) = glyph_span(&prepared.unwrap());
        assert!((0.0..2.0).contains(&left), "the start shows, at {left}");
    }

    #[test]
    fn a_textarea_scrolls_to_its_caret_but_not_back_from_the_users_scroll() {
        use crate::messages::Key;
        let lines: Vec<String> = (1..=12).map(|n| format!("satır {n}")).collect();
        let mut engine = verifying(&format!(
            "<body style='margin: 0'><textarea id=a style='font: 16px sans-serif; border: 0; padding: 0; \
             width: 200px; height: 40px; overflow: auto'>{}</textarea>",
            lines.join("\n")
        ));
        let a = engine.query(None, "#a").unwrap().unwrap();
        press(&mut engine, Key::Tab);
        let (prepared, _) = engine.prepare();
        assert_eq!(engine.verify(), Ok(()));
        let (_, y) = caret(&prepared.unwrap()).expect("a caret");
        assert!(
            (0.0..40.0).contains(&y),
            "the last line's caret at {y}, in view"
        );
        // The user scrolls the field up: its first line shows.
        let first_line = |prepared: &Prepared| {
            prepared.list.items.iter().find_map(|item| match item {
                crate::list::DisplayItem::Glyphs(run)
                    if run.text.starts_with("satır 1\n") || run.text == "satır 1" =>
                {
                    run.glyphs.first().map(|glyph| glyph.y)
                }
                _ => None,
            })
        };
        engine.wheel((0.0, -1000.0), (10.0, 10.0));
        let (prepared, _) = engine.prepare();
        assert_eq!(engine.verify(), Ok(()));
        let top = first_line(&prepared.unwrap()).expect("the first line shows");
        // Frames that do not move the caret (its blink) leave the field
        // where the user scrolled it.
        for blink in 1..=3 {
            engine.set_time(blink * 500_000_000);
            let (prepared, _) = engine.prepare();
            assert_eq!(engine.verify(), Ok(()));
            assert_eq!(
                first_line(&prepared.expect("a blink is a frame")),
                Some(top),
                "blink {blink}"
            );
        }
        assert!(engine.value(a).is_ok());
    }

    #[test]
    fn an_empty_field_shows_its_placeholder_in_grey() {
        use crate::messages::Key;
        let mut engine = verifying(
            "<input id=t placeholder='Adınız' style='color: rgb(0, 0, 255)'><button>b</button>",
        );
        let t = engine.query(None, "#t").unwrap().unwrap();
        let runs = |prepared: &Prepared| -> Vec<(String, [u8; 4])> {
            prepared
                .list
                .items
                .iter()
                .filter_map(|item| match item {
                    // The field's, not the button's.
                    crate::list::DisplayItem::Glyphs(run) if run.text != "b" => {
                        Some((run.text.clone(), run.color))
                    }
                    _ => None,
                })
                .collect()
        };
        press(&mut engine, Key::Tab);
        let (prepared, _) = engine.prepare();
        assert_eq!(engine.verify(), Ok(()));
        let prepared = prepared.unwrap();
        assert_eq!(
            runs(&prepared),
            [("Adınız".to_owned(), [117, 117, 117, 255])]
        );
        // The value is still empty, and the caret at its start.
        assert_eq!(engine.value(t), Ok(Some(String::new())));
        type_text(&mut engine, "a");
        let (prepared, _) = engine.prepare();
        assert_eq!(engine.verify(), Ok(()));
        assert_eq!(
            runs(&prepared.unwrap()),
            [("a".to_owned(), [0, 0, 255, 255])]
        );
        press(&mut engine, Key::Backspace);
        let (prepared, _) = engine.prepare();
        assert_eq!(engine.verify(), Ok(()));
        assert_eq!(runs(&prepared.unwrap())[0].0, "Adınız");
        // Its placeholder changed is laid out again.
        engine.set_attr(t, "placeholder", "Soyadınız").unwrap();
        let (prepared, _) = engine.prepare();
        assert_eq!(engine.verify(), Ok(()));
        assert_eq!(runs(&prepared.unwrap())[0].0, "Soyadınız");
    }

    fn press_with(engine: &mut Engine, key: crate::messages::Key, modifiers: Modifiers) {
        for state in [
            crate::messages::KeyState::Down,
            crate::messages::KeyState::Up,
        ] {
            let input = KeyInput {
                key: key.clone(),
                state,
                modifiers,
            };
            engine.key(&input);
            engine.key_default(&input);
        }
    }

    const SHIFT: Modifiers = Modifiers {
        shift: true,
        control: false,
        alt: false,
        meta: false,
    };
    const CONTROL: Modifiers = Modifiers {
        shift: false,
        control: true,
        alt: false,
        meta: false,
    };

    /// The selection the frame paints: its rectangles, in the selection's
    /// colour.
    fn selection(prepared: &Prepared) -> Vec<(f32, f32, f32, f32)> {
        prepared
            .list
            .items
            .iter()
            .filter_map(|item| match item {
                crate::list::DisplayItem::Rect {
                    x,
                    y,
                    width,
                    height,
                    color: [0, 120, 215, 90],
                } => Some((*x, *y, *width, *height)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn shift_and_an_arrow_select_and_what_is_typed_replaces_the_selection() {
        use crate::messages::Key;
        let mut engine = verifying("<input id=t value=abcdef><button>b</button>");
        let t = engine.query(None, "#t").unwrap().unwrap();
        press(&mut engine, Key::Tab);
        // Two characters back from the end, selected.
        press_with(&mut engine, Key::ArrowLeft, SHIFT);
        press_with(&mut engine, Key::ArrowLeft, SHIFT);
        let (prepared, _) = engine.prepare();
        assert_eq!(engine.verify(), Ok(()));
        assert_eq!(selection(&prepared.unwrap()).len(), 1, "one line selected");
        type_text(&mut engine, "X");
        assert_eq!(engine.value(t), Ok(Some("abcdX".to_owned())));
        // Home with Shift selects back to the start; without Shift an arrow
        // goes to the selection's edge.
        press_with(&mut engine, Key::Home, SHIFT);
        press(&mut engine, Key::ArrowRight);
        type_text(&mut engine, "!");
        assert_eq!(engine.value(t), Ok(Some("abcdX!".to_owned())));
        press_with(&mut engine, Key::Home, SHIFT);
        press(&mut engine, Key::ArrowLeft);
        type_text(&mut engine, "<");
        assert_eq!(engine.value(t), Ok(Some("<abcdX!".to_owned())));
        // Selected forward, from the start: the left arrow goes back to the
        // selection's start, not one step back from its end.
        press(&mut engine, Key::Home);
        press_with(&mut engine, Key::ArrowRight, SHIFT);
        press_with(&mut engine, Key::ArrowRight, SHIFT);
        press(&mut engine, Key::ArrowLeft);
        type_text(&mut engine, "|");
        assert_eq!(engine.value(t), Ok(Some("|<abcdX!".to_owned())));
        // Nothing selected after typing: no selection painted.
        let (prepared, _) = engine.prepare();
        assert_eq!(engine.verify(), Ok(()));
        assert!(selection(&prepared.unwrap()).is_empty());
    }

    #[test]
    fn control_a_selects_all_and_control_with_a_letter_types_nothing() {
        use crate::messages::Key;
        let mut engine = verifying("<input id=t value=abc><button>b</button>");
        let t = engine.query(None, "#t").unwrap().unwrap();
        press(&mut engine, Key::Tab);
        press_with(&mut engine, Key::Character("b".to_owned()), CONTROL);
        assert_eq!(
            engine.value(t),
            Ok(Some("abc".to_owned())),
            "a command, not typing"
        );
        press_with(&mut engine, Key::Character("a".to_owned()), CONTROL);
        let (prepared, _) = engine.prepare();
        assert_eq!(engine.verify(), Ok(()));
        assert_eq!(selection(&prepared.unwrap()).len(), 1);
        press(&mut engine, Key::Backspace);
        assert_eq!(engine.value(t), Ok(Some(String::new())));
        // A selection that runs past a scrolled field's edges is painted
        // inside the field.
        let mut wide = verifying(
            "<body style='margin: 0'><input id=n value='bu değer dar alana hiç sığmayacak kadar uzun' \
             style='font: 16px sans-serif; border: 0; padding: 0; width: 60px'><button>b</button>",
        );
        press(&mut wide, Key::Tab);
        press_with(&mut wide, Key::Character("a".to_owned()), CONTROL);
        let (prepared, _) = wide.prepare();
        assert_eq!(wide.verify(), Ok(()));
        let painted = selection(&prepared.unwrap());
        assert_eq!(painted.len(), 1);
        let (x, _, width, _) = painted[0];
        assert!(
            x >= 0.0 && x + width <= 60.5,
            "{painted:?} inside the 60 px field"
        );
        // AltGr, Control and Alt on Windows, types: @ on a Turkish keyboard.
        let altgr = Modifiers {
            shift: false,
            control: true,
            alt: true,
            meta: false,
        };
        press_with(&mut engine, Key::Character("@".to_owned()), altgr);
        assert_eq!(engine.value(t), Ok(Some("@".to_owned())));
    }

    #[test]
    fn a_drag_selects_and_shift_with_a_press_extends_the_selection() {
        use crate::messages::Key;
        const FIELD: &str = "style='font: 20px sans-serif; border: 0; padding: 0; width: 300px'";
        let mut engine = verifying(&format!(
            "<body style='margin: 0'><input id=t value=abcdef {FIELD}>"
        ));
        let t = engine.query(None, "#t").unwrap().unwrap();
        engine.set_value(t, "abcdef").unwrap();
        let (prepared, _) = engine.prepare();
        let xs: Vec<f32> = prepared
            .unwrap()
            .list
            .items
            .iter()
            .filter_map(|item| match item {
                crate::list::DisplayItem::Glyphs(run) if run.text == "abcdef" => {
                    Some(run.glyphs.iter().map(|glyph| glyph.x).collect::<Vec<_>>())
                }
                _ => None,
            })
            .flatten()
            .collect();
        let pointer = |engine: &mut Engine, kind, x: f32, modifiers| {
            engine.pointer(&PointerInput {
                kind,
                x,
                y: 10.0,
                button: if kind == PointerKind::Move {
                    PointerButton::None
                } else {
                    PointerButton::Primary
                },
                modifiers,
            });
        };
        // Pressed before "b", dragged to before "e": "bcd".
        pointer(
            &mut engine,
            PointerKind::Down,
            xs[1] - 1.0,
            Modifiers::default(),
        );
        pointer(
            &mut engine,
            PointerKind::Move,
            xs[4] - 1.0,
            Modifiers::default(),
        );
        pointer(
            &mut engine,
            PointerKind::Up,
            xs[4] - 1.0,
            Modifiers::default(),
        );
        let (prepared, _) = engine.prepare();
        assert_eq!(engine.verify(), Ok(()));
        let painted = selection(&prepared.unwrap());
        assert_eq!(painted.len(), 1);
        assert!(
            (painted[0].0 - xs[1]).abs() < 1.5,
            "{painted:?} from {}",
            xs[1]
        );
        type_text(&mut engine, "-");
        assert_eq!(engine.value(t), Ok(Some("a-ef".to_owned())));
        // Shift with a press: from the caret, after "-", to the end.
        pointer(&mut engine, PointerKind::Down, 290.0, SHIFT);
        pointer(&mut engine, PointerKind::Up, 290.0, SHIFT);
        press(&mut engine, Key::Delete);
        assert_eq!(engine.value(t), Ok(Some("a-".to_owned())));
        // A move with nothing pressed selects nothing.
        pointer(&mut engine, PointerKind::Move, 2.0, Modifiers::default());
        type_text(&mut engine, "!");
        assert_eq!(engine.value(t), Ok(Some("a-!".to_owned())));
    }

    #[test]
    fn copy_cut_and_paste_ask_the_clipboard_and_paste_types_its_text() {
        use crate::messages::{ClipboardRequest, Key};
        let command = |engine: &mut Engine, letter: &str| {
            press_with(engine, Key::Character(letter.to_owned()), CONTROL);
            engine.take_clipboard()
        };
        let mut engine = verifying(
            "<input id=t value=abcdef><input id=s type=password value=gizli><button>b</button>",
        );
        let (t, s) = (
            engine.query(None, "#t").unwrap().unwrap(),
            engine.query(None, "#s").unwrap().unwrap(),
        );
        press(&mut engine, Key::Tab);
        // Nothing selected: nothing to copy or cut.
        assert_eq!(command(&mut engine, "c"), None);
        assert_eq!(command(&mut engine, "x"), None);
        press_with(&mut engine, Key::ArrowLeft, SHIFT);
        press_with(&mut engine, Key::ArrowLeft, SHIFT);
        assert_eq!(
            command(&mut engine, "c"),
            Some(ClipboardRequest::Write("ef".to_owned()))
        );
        assert_eq!(
            engine.value(t),
            Ok(Some("abcdef".to_owned())),
            "copied, not cut"
        );
        assert_eq!(
            command(&mut engine, "x"),
            Some(ClipboardRequest::Write("ef".to_owned()))
        );
        assert_eq!(engine.value(t), Ok(Some("abcd".to_owned())));
        // Paste asks for the clipboard; what it holds is typed in at the
        // selection, an input event.
        press(&mut engine, Key::Home);
        press_with(&mut engine, Key::ArrowRight, SHIFT);
        assert_eq!(command(&mut engine, "v"), Some(ClipboardRequest::Read));
        let kinds: Vec<EventKind> = engine
            .paste("X\nY")
            .into_iter()
            .map(|event| event.kind)
            .collect();
        assert_eq!(kinds, [EventKind::Input]);
        assert_eq!(
            engine.value(t),
            Ok(Some("XYbcd".to_owned())),
            "an input drops line breaks"
        );
        let (prepared, _) = engine.prepare();
        assert_eq!(engine.verify(), Ok(()));
        assert!(caret(&prepared.unwrap()).is_some());
        // A password is never copied or cut.
        press(&mut engine, Key::Tab);
        press_with(&mut engine, Key::Character("a".to_owned()), CONTROL);
        assert_eq!(command(&mut engine, "c"), None);
        assert_eq!(command(&mut engine, "x"), None);
        assert_eq!(engine.value(s), Ok(Some("gizli".to_owned())));
        // With no field focused, a paste types nothing.
        press(&mut engine, Key::Tab);
        assert_eq!(engine.paste("z"), Vec::new());
    }

    #[test]
    fn leaving_a_field_the_user_changed_is_a_change_event() {
        use crate::messages::Key;
        let mut engine = verifying("<input id=t value=a><input id=u value=b>");
        press(&mut engine, Key::Tab);
        // Not changed: no change event.
        assert_eq!(
            press(&mut engine, Key::Tab),
            [
                EventKind::KeyDown,
                EventKind::Blur,
                EventKind::Focus,
                EventKind::KeyUp
            ]
        );
        type_text(&mut engine, "x");
        assert_eq!(
            press(&mut engine, Key::Tab),
            [
                EventKind::KeyDown,
                EventKind::Change,
                EventKind::Blur,
                EventKind::Focus,
                EventKind::KeyUp
            ]
        );
        // Changed by the host, not the user: no change event.
        let u = engine.query(None, "#u").unwrap().unwrap();
        press(&mut engine, Key::Tab);
        engine.set_value(u, "host").unwrap();
        assert_eq!(
            press(&mut engine, Key::Tab),
            [
                EventKind::KeyDown,
                EventKind::Blur,
                EventKind::Focus,
                EventKind::KeyUp
            ]
        );
        // Changed and changed back: the value is as it was.
        type_text(&mut engine, "y");
        press(&mut engine, Key::Backspace);
        assert_eq!(
            press(&mut engine, Key::Tab),
            [
                EventKind::KeyDown,
                EventKind::Blur,
                EventKind::Focus,
                EventKind::KeyUp
            ]
        );
    }

    #[test]
    fn a_paragraph_moved_by_a_fraction_of_a_pixel_is_measured_again() {
        // Nothing in the paragraph's box changes, and Taffy does not lay it
        // out again; but its rounded width follows where it lands, and a
        // relative offset in percent follows the width.
        let mut engine = verifying(
            "<div id=g style='margin-left: 0.3px'><div style='width: 100px'>\
             <p style='width: 50.5px'>a <span style='position: relative; left: 50%'>b</span></p>\
             </div></div>",
        );
        let g = engine.query(None, "#g").unwrap().unwrap();
        for margin in ["0.6px", "0.2px", "0.9px", "0.4px"] {
            engine.set_style_property(g, "margin-left", margin).unwrap();
            boxes_laid_out(&mut engine);
        }
    }

    #[test]
    fn a_float_that_changes_moves_the_text_beside_it() {
        // Taffy's cache keys a box by its inputs, not by the floats around
        // it; the oracle checks that the boxes beside a changed float are
        // laid out again.
        let mut engine = verifying(
            "<div style='width: 200px'><div id=f style='float: left; width: 60px; height: 20px'></div>             <p>yanındaki metin uzun bir satır olarak akıyor</p><p id=q>ikinci paragraf</p></div>",
        );
        let f = engine.query(None, "#f").unwrap().unwrap();
        for height in ["60px", "5px", "100px"] {
            engine.set_style_property(f, "height", height).unwrap();
            boxes_laid_out(&mut engine);
        }
        engine.set_style_property(f, "float", "right").unwrap();
        boxes_laid_out(&mut engine);
    }

    #[test]
    fn another_image_known_already_lays_out_again() {
        let mut engine = Engine::new();
        engine.load_html("<img id=a src=a.png><img id=b src=b.png>");
        engine.resize(300, 200);
        engine.set_verifying(true);
        let (_, requests) = engine.prepare();
        for request in requests {
            // The same ratio: the style is the same, the natural size not.
            let size = if request.url == "a.png" {
                (2, 1)
            } else {
                (40, 20)
            };
            engine.complete_resource(&ResourceResponse {
                id: request.id,
                mime: "image/png".to_owned(),
                data: crate::resources::tests::png_of(size.0, size.1),
            });
        }
        boxes_laid_out(&mut engine);
        // Both have arrived: nothing arrives now, the box changes.
        let a = engine.query(None, "#a").unwrap().unwrap();
        engine.set_attr(a, "src", "b.png").unwrap();
        boxes_laid_out(&mut engine);
    }

    #[test]
    fn what_arrives_shapes_every_paragraph_again() {
        let mut engine = Engine::new();
        engine.load_html("<p>bir</p><p>iki</p><img src=a.png>");
        engine.resize(300, 200);
        engine.set_verifying(true);
        let (_, requests) = engine.prepare();
        let request = requests
            .iter()
            .find(|r| r.url == "a.png")
            .expect("a.png is asked for");
        engine.complete_resource(&ResourceResponse {
            id: request.id,
            mime: "image/png".to_owned(),
            data: crate::resources::tests::tiny_png(),
        });
        // Fonts may have arrived too: every paragraph, the image's
        // anonymous one with them.
        assert!(laid_out(&mut engine));
        assert_eq!(engine.stats().shaped, 3);
    }

    #[test]
    fn an_inline_element_is_painted_in_its_own_colour() {
        let engine =
            verifying("<p style='color: black'>x <span style='color: red'>kırmızı</span> y</p>");
        let (list, ..) = engine.last.as_ref().expect("verifying keeps the frame");
        let painted = |color: [u8; 4]| -> String {
            list.items
                .iter()
                .filter_map(|item| match item {
                    crate::list::DisplayItem::Glyphs(run) if run.color == color => {
                        Some(run.text.clone())
                    }
                    _ => None,
                })
                .collect()
        };
        assert_eq!(painted([255, 0, 0, 255]).trim(), "kırmızı");
        assert_eq!(painted([0, 0, 0, 255]).replace(' ', ""), "xy");
    }

    #[test]
    fn a_frame_that_only_repaints_keeps_the_layout() {
        let mut engine = verifying(
            "<style>li:hover { color: red } .on { background: yellow }</style>             <p id=p>bir <span id=s style='background: #eee'>iki</span> üç</p>             <ul><li id=li>dört</li></ul>",
        );
        let find = |engine: &Engine, selector| engine.query(None, selector).unwrap().unwrap();
        let (p, s, li) = (
            find(&engine, "#p"),
            find(&engine, "#s"),
            find(&engine, "#li"),
        );
        // A text colour (B13), a block's background, a class.
        engine.set_style_property(p, "color", "blue").unwrap();
        assert!(engine.prepare().0.is_some());
        assert_eq!(engine.verify(), Ok(()));
        assert_eq!(engine.stats().laid_out, 0);
        // The kept text is painted in the frame's colour.
        let (list, ..) = engine.last.as_ref().expect("verifying keeps the frame");
        let blue = list.items.iter().any(|item| {
            matches!(item, crate::list::DisplayItem::Glyphs(run)
                if run.text.contains("bir") && run.color == [0, 0, 255, 255])
        });
        assert!(blue, "the text is not blue");
        engine.set_attr(li, "class", "on").unwrap();
        assert!(!laid_out(&mut engine));
        // An inline background that was painted before.
        engine.set_style_property(s, "background", "red").unwrap();
        assert!(!laid_out(&mut engine));
        // The pointer over an element whose colour follows it.
        let li_box = engine.node_box(li).unwrap().unwrap();
        engine.pointer(&PointerInput {
            kind: PointerKind::Move,
            x: li_box.x + 2.0,
            y: li_box.y + 2.0,
            button: PointerButton::Primary,
            modifiers: Modifiers::default(),
        });
        assert!(!laid_out(&mut engine));
    }

    #[test]
    fn a_colour_that_splits_the_text_otherwise_lays_it_out_again() {
        // A colour splits glyph runs, and fonts are picked per run: a
        // colour may change without layout only where the text stays split
        // as it was.
        let mut engine = verifying("<p>bir <span id=s>iki</span> üç</p>");
        let s = engine.query(None, "#s").unwrap().unwrap();
        // Like its neighbours before, unlike them now.
        engine.set_style_property(s, "color", "red").unwrap();
        assert!(laid_out(&mut engine));
        // Unlike them before and after.
        engine.set_style_property(s, "color", "blue").unwrap();
        assert!(!laid_out(&mut engine));
        // Like them again.
        engine.set_style_property(s, "color", "black").unwrap();
        assert!(laid_out(&mut engine));
    }

    #[test]
    fn a_joiner_shaped_with_its_neighbour_keeps_its_colour() {
        // The joiner at the start of the span shapes with the letter before
        // it, so the run in the span's colour holds text of the paragraph's:
        // it is still painted in the span's colour.
        let engine = verifying(
            "<div dir=rtl style='font-size: 40px'>\u{639}\u{200d}<span style='color: blue'>\u{200d}\u{639}\u{200d}</span>\u{200d}\u{639}</div>",
        );
        let (list, ..) = engine.last.as_ref().expect("verifying keeps the frame");
        let colours: Vec<(String, [u8; 4])> = list
            .items
            .iter()
            .filter_map(|item| match item {
                crate::list::DisplayItem::Glyphs(run) => Some((run.text.clone(), run.color)),
                _ => None,
            })
            .collect();
        let (black, blue) = ([0, 0, 0, 255], [0, 0, 255, 255]);
        assert_eq!(
            colours,
            [
                ("\u{639}".to_owned(), black),
                ("\u{200d}".to_owned(), blue),
                ("\u{200d}\u{639}\u{200d}".to_owned(), black)
            ]
        );
    }

    #[test]
    fn a_frame_that_changes_the_layout_lays_out_again() {
        let mut engine =
            verifying("<p id=p lang=en>bir <span id=s>iki</span></p><img id=img src=a.png>");
        let find = |engine: &Engine, selector| engine.query(None, selector).unwrap().unwrap();
        let (p, s, img) = (
            find(&engine, "#p"),
            find(&engine, "#s"),
            find(&engine, "#img"),
        );
        engine.set_style_property(p, "width", "50px").unwrap();
        assert!(laid_out(&mut engine));
        // An inline background where none was painted: the paragraph gets
        // a decoration.
        engine.set_style_property(s, "background", "red").unwrap();
        assert!(laid_out(&mut engine));
        // Hidden: its decoration goes.
        engine
            .set_style_property(s, "visibility", "hidden")
            .unwrap();
        assert!(laid_out(&mut engine));
        engine.set_text(s, "üç").unwrap();
        assert!(laid_out(&mut engine));
        // A text node's data, its element's children unchanged.
        let text = engine.child_at(s, 0).unwrap().unwrap();
        engine.set_text(text, "beş").unwrap();
        assert!(laid_out(&mut engine));
        // Attributes layout reads without styles: the language, the image.
        engine.set_attr(p, "lang", "tr").unwrap();
        assert!(laid_out(&mut engine));
        engine.set_attr(img, "src", "b.png").unwrap();
        let (_, requests) = engine.prepare();
        assert_eq!(engine.verify(), Ok(()));
        // An image arriving changes the layout without any change to the
        // document.
        let request = requests
            .iter()
            .find(|r| r.url == "b.png")
            .expect("b.png is asked for");
        engine.complete_resource(&ResourceResponse {
            id: request.id,
            mime: "image/png".to_owned(),
            data: crate::resources::tests::tiny_png(),
        });
        assert!(laid_out(&mut engine));
        // A viewport of another size.
        engine.resize(200, 200);
        assert!(laid_out(&mut engine));
    }

    #[test]
    fn the_deepest_document_runs_on_a_main_threads_stack() {
        // A stack overflow aborts the process: the test fails loudly, not
        // by a caught panic.
        let html = format!(
            "<body>{}derin{}",
            "<div>".repeat(5000),
            "</div>".repeat(5000)
        );
        std::thread::Builder::new()
            .stack_size(MAIN_THREAD_STACK)
            .spawn(move || {
                let mut engine = Engine::new();
                engine.load_html(&html);
                engine.resize(200, 100);
                let (prepared, _) = engine.prepare();
                assert!(prepared.is_some());
                let div = engine.query(None, "div").unwrap().unwrap();
                assert_eq!(engine.text(div).unwrap(), "derin");
                // Matching a descendant combinator walks every ancestor; the
                // parser's cap at 512 levels makes the rest siblings.
                let deeper = engine.query(None, "body div div div:not(:first-child)");
                assert!(matches!(deeper, Ok(Some(_))), "{deeper:?}");
                engine.pointer(&PointerInput {
                    kind: PointerKind::Move,
                    x: 10.0,
                    y: 10.0,
                    button: PointerButton::None,
                    modifiers: Modifiers::default(),
                });
                assert!(engine.inspect_at(10.0, 10.0).is_some());
                engine.set_text(div, "sığ").unwrap();
                let (prepared, _) = engine.prepare();
                assert!(prepared.is_some());
                engine.load_html("<p>sonra</p>");
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn every_frame_of_every_engine_is_prepared_on_one_thread() {
        // A thread per frame would leave Stylo's thread-local caches behind
        // each time; one thread keeps them for the next frame.
        let mut threads = Vec::new();
        for _ in 0..2 {
            let mut engine = Engine::new();
            engine.load_html("<p>Erk</p>");
            for width in [100, 120, 140] {
                engine.resize(width, 50);
                assert!(engine.prepare().0.is_some());
                threads.push(engine.frame_thread.expect("a frame was prepared"));
            }
        }
        assert!(
            threads.iter().all(|thread| *thread == threads[0]),
            "{threads:?}"
        );
        assert_ne!(threads[0], std::thread::current().id());
    }

    #[test]
    fn a_frame_is_prepared_only_after_a_change_that_shows() {
        let mut engine = Engine::new();
        engine.load_html("<p>Erk</p>");
        assert!(engine.prepare().0.is_none(), "no viewport yet");
        engine.resize(100, 50);
        assert!(engine.prepare().0.is_some());
        assert!(engine.prepare().0.is_none(), "nothing changed");
        let p = engine.query(None, "p").unwrap().unwrap();
        assert_eq!(engine.set_text(p + (1 << 32), "x"), Err(Status::StaleNode));
        assert!(
            engine.prepare().0.is_none(),
            "a failed change shows nothing"
        );
        engine.set_text(p, "Erk!").unwrap();
        assert!(engine.prepare().0.is_some());
    }

    #[test]
    fn a_removed_element_holds_no_state_the_next_input_reports() {
        // Focused, hovered and pressed, then removed: the next input must
        // not report it (a blur for a node the host can no longer use).
        let mut engine = Engine::new();
        engine.load_html(
            r#"<body style="margin: 0"><div id=a tabindex=0 style="height: 40px"></div><div id=b tabindex=0 style="height: 40px"></div>"#,
        );
        engine.resize(100, 100);
        engine.prepare();
        let a = engine.query(None, "#a").unwrap().unwrap();
        let tab = KeyInput {
            key: crate::messages::Key::Tab,
            state: crate::messages::KeyState::Down,
            modifiers: Modifiers::default(),
        };
        // The key event and what the key does by default.
        let press = |engine: &mut Engine| {
            let mut events: Vec<Event> = engine.key(&tab).into_iter().collect();
            events.extend(engine.key_default(&tab));
            events
        };
        let focused = press(&mut engine);
        assert!(focused.iter().any(|event| event.target == a), "{focused:?}");
        engine.pointer(&PointerInput {
            kind: PointerKind::Down,
            x: 10.0,
            y: 10.0,
            button: PointerButton::Primary,
            modifiers: Modifiers::default(),
        });
        engine.remove(a).unwrap();
        engine.prepare();
        let events = press(&mut engine);
        assert!(events.iter().all(|event| event.target != a), "{events:?}");
        let released = engine.pointer(&PointerInput {
            kind: PointerKind::Up,
            x: 10.0,
            y: 50.0,
            button: PointerButton::Primary,
            modifiers: Modifiers::default(),
        });
        assert!(
            released.iter().all(|event| event.target != a),
            "{released:?}"
        );
    }

    #[test]
    fn text_reads_as_text_content() {
        let mut engine = Engine::new();
        engine.load_html("<div id=a>bir <b>iki</b><!-- yok --> üç</div>");
        let a = engine.query(None, "#a").unwrap().unwrap();
        assert_eq!(engine.text(a).unwrap(), "bir iki üç");
        assert_eq!(engine.text(engine.root()).unwrap(), "");
        assert_eq!(engine.text(0), Err(Status::InvalidArgument));
        engine.load_html("<p>yeni</p>");
        assert_eq!(engine.text(a), Err(Status::StaleNode));
    }

    const FIELD: &str = "font: 20px sans-serif; border: 0; padding: 0; margin: 0";

    /// A focused field styled `style`, in a box that clips it, its frame
    /// made: the engine and that
    /// frame's display list.
    fn focused_field(style: &str) -> (Engine, Vec<crate::list::DisplayItem>) {
        let mut engine = verifying(&format!(
            "<body style='margin: 20px'><div style='overflow: hidden; padding: 50px'><input value=ab style='{FIELD}; {style}'></div><button>b</button>"
        ));
        assert_eq!(engine.caret_rect(), None, "no focus, no caret");
        press(&mut engine, crate::messages::Key::Tab);
        let (prepared, _) = engine.prepare();
        assert_eq!(engine.verify(), Ok(()));
        (engine, prepared.expect("a frame").list.items)
    }

    #[test]
    fn the_caret_rect_is_where_the_caret_is_painted_and_goes_with_the_focus() {
        let (mut engine, items) = focused_field("");
        let rect = engine.caret_rect().expect("a caret");
        let painted = items
            .iter()
            .find_map(|item| match item {
                crate::list::DisplayItem::Rect {
                    x,
                    y,
                    width: 1.0,
                    height,
                    ..
                } => Some(CaretRect {
                    x: *x,
                    y: *y,
                    width: 1.0,
                    height: *height,
                }),
                _ => None,
            })
            .expect("a painted caret");
        assert_eq!(rect, painted);
        // At the end of "ab", on the line of a 20px font.
        assert!(rect.x > 30.0 && rect.height > 15.0, "{rect:?}");
        // Blinking off, the caret is not painted but is still there.
        engine.set_time(CARET_BLINK_NS);
        let (prepared, _) = engine.prepare();
        assert_eq!(engine.verify(), Ok(()));
        assert_eq!(caret(&prepared.expect("a frame")), None);
        assert_eq!(engine.caret_rect(), Some(rect));
        // Leaving the field, it goes.
        press(&mut engine, crate::messages::Key::Tab);
        assert!(engine.prepare().0.is_some());
        assert_eq!(engine.caret_rect(), None);
    }

    const CARET_BLINK_NS: u64 = 500_000_000;

    /// Whether `items` paints `wanted` inside a transform: after a push of
    /// one that is not yet popped.
    fn inside_a_transform(
        items: &[crate::list::DisplayItem],
        wanted: impl Fn(&crate::list::DisplayItem) -> bool,
    ) -> bool {
        let mut open = 0;
        for item in items {
            match item {
                crate::list::DisplayItem::PushTransform(_) => open += 1,
                crate::list::DisplayItem::PopTransform => open -= 1,
                item if wanted(item) => return open > 0,
                _ => {}
            }
        }
        panic!("not painted")
    }

    fn is_caret(item: &crate::list::DisplayItem) -> bool {
        matches!(item, crate::list::DisplayItem::Rect { width: 1.0, .. })
    }

    fn is_selection(item: &crate::list::DisplayItem) -> bool {
        matches!(item, crate::list::DisplayItem::Rect { color, .. } if color[3] == 90)
    }

    #[test]
    fn a_transformed_fields_caret_is_painted_and_reported_where_the_field_went() {
        let (plain, items) = focused_field("");
        let at = plain.caret_rect().expect("a caret");
        assert!(!inside_a_transform(&items, is_caret));
        // Moved: the caret goes with it, painted in the field's transform.
        let (mut moved, items) = focused_field("transform: translate(40px, 30px)");
        let rect = moved.caret_rect().expect("a caret");
        assert!(
            (rect.x - at.x - 40.0).abs() < 0.01 && (rect.y - at.y - 30.0).abs() < 0.01,
            "{rect:?} from {at:?}"
        );
        assert_eq!((rect.width, rect.height), (at.width, at.height));
        assert!(inside_a_transform(&items, is_caret));
        // So is the selection.
        press_with(
            &mut moved,
            crate::messages::Key::Character("a".to_owned()),
            CONTROL,
        );
        let (prepared, _) = moved.prepare();
        assert_eq!(moved.verify(), Ok(()));
        assert!(inside_a_transform(
            &prepared.expect("a frame").list.items,
            is_selection
        ));
        // Turned a quarter: the caret lies across, and its box with it.
        let (turned, _) = focused_field("transform: rotate(90deg)");
        let rect = turned.caret_rect().expect("a caret");
        assert!(
            (rect.width - at.height).abs() < 0.01 && (rect.height - 1.0).abs() < 0.01,
            "{rect:?} from {at:?}"
        );
    }

    /// The text a frame paints in blue, its runs joined, and the underlines
    /// it draws: rectangles a pixel high in that blue.
    fn shown(prepared: &Prepared) -> (String, usize) {
        let text = prepared
            .list
            .items
            .iter()
            .filter_map(|item| match item {
                crate::list::DisplayItem::Glyphs(run) if run.color == [0, 0, 255, 255] => {
                    Some(run.text.as_str())
                }
                _ => None,
            })
            .collect();
        let underlines = prepared
            .list
            .items
            .iter()
            .filter(|item| {
                matches!(item, crate::list::DisplayItem::Rect { height: 1.0, width, color: [0, 0, 255, 255], .. } if *width > 1.0)
            })
            .count();
        (text, underlines)
    }

    /// A frame, checked against the oracle.
    fn frame(engine: &mut Engine) -> Prepared {
        let (prepared, _) = engine.prepare();
        assert_eq!(engine.verify(), Ok(()));
        prepared.expect("a frame")
    }

    const BLUE_FIELD: &str = "<input id=t value=ab style='color: rgb(0, 0, 255); font: 20px sans-serif'><button>b</button>";

    #[test]
    fn an_input_method_composes_at_the_caret_and_commits_into_the_value() {
        use crate::messages::Key;
        let mut engine = verifying(BLUE_FIELD);
        let t = engine.query(None, "#t").unwrap().unwrap();
        press(&mut engine, Key::Tab);
        press(&mut engine, Key::ArrowLeft);
        let before = caret(&frame(&mut engine)).expect("a caret");
        assert!(engine.takes_composition());
        // Shown and underlined at the caret, not in the value.
        assert_eq!(engine.compose("か", None), []);
        let prepared = frame(&mut engine);
        assert_eq!(shown(&prepared), ("aかb".to_owned(), 1));
        assert_eq!(engine.value(t), Ok(Some("ab".to_owned())));
        // The caret at the composition's end; the input method's window
        // at its start, where the caret was.
        let after = caret(&prepared).expect("a caret");
        assert!(after.0 > before.0 + 5.0, "{after:?} after {before:?}");
        assert_eq!(engine.caret_rect().map(|rect| rect.x), Some(before.0));
        // The caret inside it, where the input method puts it.
        engine.compose("かな", Some("か".len()));
        let prepared = frame(&mut engine);
        assert_eq!(shown(&prepared).0, "aかなb");
        let inside = caret(&prepared).expect("a caret");
        assert!((inside.0 - after.0).abs() < 1.0, "{inside:?} and {after:?}");
        // Keys are the input method's while it composes: none edits.
        assert_eq!(
            press(&mut engine, Key::Character("x".to_owned())),
            [EventKind::KeyDown, EventKind::KeyUp]
        );
        press(&mut engine, Key::Backspace);
        assert_eq!(engine.value(t), Ok(Some("ab".to_owned())));
        // Committed: typed in as a paste, an input event.
        let kinds: Vec<EventKind> = engine
            .commit_composition("仮名")
            .into_iter()
            .map(|event| event.kind)
            .collect();
        assert_eq!(kinds, [EventKind::Input]);
        assert_eq!(engine.value(t), Ok(Some("a仮名b".to_owned())));
        assert_eq!(shown(&frame(&mut engine)), ("a仮名b".to_owned(), 0));
        // Typing edits again, at the caret after what was committed.
        type_text(&mut engine, "!");
        assert_eq!(engine.value(t), Ok(Some("a仮名!b".to_owned())));
    }

    #[test]
    fn a_composition_takes_the_selections_place_and_ends_with_the_focus_or_a_press() {
        use crate::messages::Key;
        let mut engine = verifying(BLUE_FIELD);
        let t = engine.query(None, "#t").unwrap().unwrap();
        press(&mut engine, Key::Tab);
        press_with(&mut engine, Key::Character("a".to_owned()), CONTROL);
        // Starting over a selection deletes it, an input event.
        let kinds: Vec<EventKind> = engine
            .compose("k", None)
            .into_iter()
            .map(|event| event.kind)
            .collect();
        assert_eq!(kinds, [EventKind::Input]);
        assert_eq!(engine.value(t), Ok(Some(String::new())));
        assert_eq!(shown(&frame(&mut engine)), ("k".to_owned(), 1));
        // An empty composition ends it.
        engine.compose("", None);
        assert_eq!(shown(&frame(&mut engine)), (String::new(), 0));
        // The focus leaving takes it away; the value stays.
        engine.compose("k", None);
        frame(&mut engine);
        press(&mut engine, Key::Tab);
        assert_eq!(shown(&frame(&mut engine)), (String::new(), 0));
        // A press in the field ends it too.
        press(&mut engine, Key::Tab);
        engine.compose("k", None);
        let field = engine.node_box(t).unwrap().unwrap();
        click(&mut engine, (field.x + 2.0, field.y + field.height / 2.0));
        assert_eq!(shown(&frame(&mut engine)), (String::new(), 0));
        assert_eq!(engine.value(t), Ok(Some(String::new())));
    }

    #[test]
    fn a_press_past_a_composition_finds_its_place_in_the_value() {
        use crate::messages::Key;
        let mut engine = verifying(
            "<input id=t value=abcdef style='color: rgb(0, 0, 255); font: 20px sans-serif; border: 0; padding: 0; width: 290px'><button>b</button>",
        );
        let t = engine.query(None, "#t").unwrap().unwrap();
        press(&mut engine, Key::Tab);
        press(&mut engine, Key::Home);
        press(&mut engine, Key::ArrowRight);
        frame(&mut engine);
        // Where "c" ends, before anything is composed.
        for _ in 0..2 {
            press(&mut engine, Key::ArrowRight);
        }
        let after_c = caret(&frame(&mut engine)).expect("a caret").0;
        for _ in 0..2 {
            press(&mut engine, Key::ArrowLeft);
        }
        // Composed after "a", what follows moves right by its width.
        engine.compose("かな", None);
        let end = caret(&frame(&mut engine)).expect("a caret").0;
        let width = end - engine.caret_rect().expect("a caret").x;
        let y = engine.caret_rect().unwrap().y + 5.0;
        // A press where "c" now ends: there in the value, the composition
        // gone.
        click(&mut engine, (after_c + width + 0.5, y));
        frame(&mut engine);
        type_text(&mut engine, "!");
        assert_eq!(engine.value(t), Ok(Some("abc!def".to_owned())));
    }

    #[test]
    fn only_a_text_field_that_is_no_password_takes_a_composition() {
        use crate::messages::Key;
        let mut engine = verifying(
            "<input id=p type=password value=gizli style='color: rgb(255, 0, 0)'>\
             <input id=t placeholder=Adınız style='color: rgb(0, 0, 255)'>\
             <textarea id=a style='color: rgb(0, 0, 255)'></textarea><button>b</button>",
        );
        let (p, t, a) = (
            engine.query(None, "#p").unwrap().unwrap(),
            engine.query(None, "#t").unwrap().unwrap(),
            engine.query(None, "#a").unwrap().unwrap(),
        );
        assert!(!engine.takes_composition());
        assert_eq!(engine.compose("k", None), []);
        press(&mut engine, Key::Tab);
        assert!(!engine.takes_composition());
        assert_eq!(engine.compose("k", None), []);
        let lines = frame(&mut engine)
            .list
            .items
            .iter()
            .filter(|item| {
                matches!(item, crate::list::DisplayItem::Rect { height: 1.0, width, .. } if *width > 1.0)
            })
            .count();
        assert_eq!(lines, 0, "nothing composed, nothing underlined");
        // Text committed is typed, as in any field.
        engine.commit_composition("k");
        assert_eq!(engine.value(p), Ok(Some("gizlik".to_owned())));
        // A line takes no line break; the placeholder goes while it
        // composes.
        press(&mut engine, Key::Tab);
        assert!(engine.takes_composition());
        engine.compose("bir\niki", None);
        assert_eq!(shown(&frame(&mut engine)), ("biriki".to_owned(), 1));
        engine.commit_composition("bir\niki");
        assert_eq!(engine.value(t), Ok(Some("biriki".to_owned())));
        // A textarea keeps it.
        press(&mut engine, Key::Tab);
        engine.commit_composition("bir\niki");
        assert_eq!(engine.value(a), Ok(Some("bir\niki".to_owned())));
    }

    #[test]
    fn size_cols_and_rows_size_a_text_field_in_characters_and_lines() {
        const PLAIN: &str = "font: 16px monospace; line-height: 20px; border: 0; padding: 0";
        let mut engine = verifying(&format!(
            "<input id=a style='{PLAIN}'><input id=b size=40 style='{PLAIN}'>\
             <input id=c size=0 style='{PLAIN}'><input id=d size=' +5x' style='{PLAIN}'>\
             <input id=e type=checkbox size=40>\
             <textarea id=t cols=10 rows=3 style='{PLAIN}'></textarea>\
             <textarea id=u cols=-4 rows=two style='{PLAIN}'></textarea>"
        ));
        let size = |engine: &Engine, id: &str| {
            let node = engine.query(None, &format!("#{id}")).unwrap().unwrap();
            let found = engine.node_box(node).unwrap().unwrap();
            (found.width, found.height)
        };
        let (twenty, _) = size(&engine, "a");
        // Twenty characters by default, as many as `size` says if it is a
        // positive integer (layout rounds to whole pixels).
        assert!(twenty > 100.0, "{twenty}");
        assert!((size(&engine, "b").0 - 2.0 * twenty).abs() < 1.0);
        assert_eq!(size(&engine, "c").0, twenty);
        assert!((size(&engine, "d").0 - twenty / 4.0).abs() < 1.0);
        // Not on a checkbox.
        let e = engine.query(None, "#e").unwrap().unwrap();
        let style = engine.computed_style(e).unwrap().unwrap();
        // Its own width line, not `min-width`'s.
        assert!(
            style.lines().any(|line| line.trim() == "width: auto;"),
            "{style}"
        );
        // A textarea: `cols` characters wide, `rows` lines high.
        let (width, height) = size(&engine, "t");
        assert!((width - twenty / 2.0).abs() < 1.0, "{width}");
        assert_eq!(height, 60.0);
        // Values that are no positive integer keep the defaults: twenty
        // characters, two lines.
        assert_eq!(size(&engine, "u"), (twenty, 40.0));
        // Set later, they size the field again; the frame is the oracle's.
        let t = engine.query(None, "#t").unwrap().unwrap();
        engine.set_attr(t, "rows", "5").unwrap();
        engine.set_attr(t, "cols", "20").unwrap();
        assert!(engine.prepare().0.is_some());
        assert_eq!(engine.verify(), Ok(()));
        assert_eq!(size(&engine, "t"), (twenty, 100.0));
        engine.remove_attr(t, "rows").unwrap();
        assert!(engine.prepare().0.is_some());
        assert_eq!(engine.verify(), Ok(()));
        assert_eq!(size(&engine, "t").1, 40.0);
    }

    #[test]
    fn a_presentational_attribute_set_later_restyles_its_element() {
        let mut engine = verifying("<div id=d style='height: 10px'>x</div>");
        let d = engine.query(None, "#d").unwrap().unwrap();
        engine.set_attr(d, "bgcolor", "#ff0000").unwrap();
        assert!(engine.prepare().0.is_some());
        assert_eq!(engine.verify(), Ok(()));
        let style = engine.computed_style(d).unwrap().unwrap();
        assert!(
            style.contains("background-color: rgb(255, 0, 0)"),
            "{style}"
        );
    }
}
