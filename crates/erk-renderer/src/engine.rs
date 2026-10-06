//! The engine (p1-contract §1.1): the document, its resources, the user's
//! input and the preparation of each frame, on the caller's thread. Changes
//! apply at once and questions are answered at once; painting is the
//! raster's (raster.rs), which gets a [`Prepared`] frame and nothing else.
//!
//! The embedding layer (`erk`) keeps an engine on its UI thread. A frame's
//! style, layout and display list run on a helper thread with a large stack
//! while the caller waits: layout recurses once per level of nesting, and
//! the deepest document the parser builds needs more stack than a UI
//! thread has (1 MiB on Windows; the measurement is in the M3 plan).

use erk_dom::NodeId;

use crate::list::Prepared;
use crate::messages::{
    BoxModel, Cursor, Event, FontCatalog, KeyInput, NodeKind, PointerInput, ResourceRequest,
    ResourceResponse, Stage, Status,
};
use crate::page::Page;
use crate::resources::Resources;

/// The stack a frame is prepared on: the deepest document (512 levels)
/// needs 2 to 4 MiB in a release build and 4 to 8 in a debug one. It is
/// address space, committed only as it is used.
pub(crate) const FRAME_STACK: usize = 16 * 1024 * 1024;

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

    /// A key went down or up while the page has the keyboard.
    pub fn key(&mut self, input: &KeyInput) -> Vec<Event> {
        self.restyled_by(|page| page.key(input))
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

    /// Set `node`'s text as `textContent` does.
    pub fn set_text(&mut self, node: u64, text: &str) -> Result<(), Status> {
        let result = self.page.set_text(node, text);
        self.changed |= result.is_ok();
        result
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

    /// [`Engine::prepare`], calling `mark` as each stage ends, on the
    /// thread that runs them: the embedding layer times the stages, the
    /// core reads no clock (p1-contract §8.1).
    pub fn prepare_marked(
        &mut self,
        mark: &mut (dyn FnMut(Stage) + Send),
    ) -> (Option<Prepared>, Vec<ResourceRequest>) {
        let Some((width, height)) = self.size.filter(|_| self.changed) else {
            return (None, Vec::new());
        };
        let scale = self.scale;
        let (page, resources) = (&mut self.page, &mut self.resources);
        let (list, requests) = std::thread::scope(|scope| {
            std::thread::Builder::new()
                .name("erk-frame".to_owned())
                .stack_size(FRAME_STACK)
                .spawn_scoped(scope, || {
                    page.prepare(width, height, scale, resources, mark)
                })
                .expect("a frame thread starts")
                .join()
                .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
        });
        let updates = self.resources.table_updates(&list);
        self.changed = false;
        let prepared = Prepared {
            updates,
            list,
            width,
            height,
            scale: crate::device_scale(scale),
            pending: self.resources.pending(),
        };
        (Some(prepared), requests)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::{Modifiers, PointerButton, PointerKind};

    /// The stack Windows gives a program's main thread, where a host's UI
    /// thread usually is.
    const MAIN_THREAD_STACK: usize = 1024 * 1024;

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
