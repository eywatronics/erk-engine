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
    BoxModel, Cursor, Event, FontCatalog, FrameStats, KeyInput, NodeKind, PointerInput,
    ResourceRequest, ResourceResponse, Stage, Status,
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
        self.restyled_by(|page| page.pointer(input))
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
        self.restyled_by(|page| page.key_default(input))
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
    use crate::messages::{Modifiers, PointerButton, PointerKind};

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
}
