//! What a callback may do: everything an app does but run its loop or end
//! (p1-contract §5). An [`crate::App`] dereferences to its context, so the
//! host and its callbacks use one API; a callback gets `&mut Context` and
//! so cannot tick, run or drop the app it is called from.

use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::mpsc::Receiver;

use erk_renderer::{BoxModel, Engine, FrameStats, NodeKind};

use crate::Status;
use crate::events::{self, Event, EventKind, Listener, Listeners, Phase, Subscription};
use crate::handle::{AppHandle, Inbox, Message, Responder, Waker};
use crate::ids::{self, Node};

/// A node a mutation names: one the host has, or one an earlier mutation of
/// the same batch created (its position in the batch).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ref {
    Node(Node),
    New(usize),
}

/// One change of a batch ([`Context::apply`]); each does what the method of
/// the same name does.
#[derive(Clone, Debug, PartialEq)]
pub enum Mutation {
    CreateElement(String),
    CreateText(String),
    Append {
        parent: Ref,
        child: Ref,
    },
    InsertBefore {
        parent: Ref,
        child: Ref,
        before: Option<Ref>,
    },
    Remove(Ref),
    SetText(Ref, String),
    SetAttr(Ref, String, String),
    RemoveAttr(Ref, String),
    AddClass(Ref, String),
    RemoveClass(Ref, String),
}

/// Why a batch stopped: the mutation at `index` failed with `status`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BatchError {
    pub index: usize,
    pub status: Status,
}

impl std::fmt::Display for BatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "mutation {} failed: {}", self.index, self.status)
    }
}

impl std::error::Error for BatchError {}

pub struct Context {
    pub(crate) engine: Engine,
    /// The key this app's node ids are mixed with (p1-contract §2).
    key: u64,
    listeners: Listeners,
    /// Set by [`Context::stop_propagation`] during a dispatch.
    stopped: bool,
    /// The way in for other threads, which handles copy, and what they sent.
    entry: Inbox,
    inbox: Receiver<Message>,
    /// Messages taken off the inbox, not handled yet.
    queued: VecDeque<Message>,
}

impl Context {
    pub(crate) fn new(engine: Engine, key: u64) -> Self {
        let (to, inbox) = std::sync::mpsc::channel();
        Self {
            engine,
            key,
            listeners: Listeners::default(),
            stopped: false,
            entry: Inbox {
                to,
                waker: Waker::default(),
            },
            inbox,
            queued: VecDeque::new(),
        }
    }

    /// Show `html` instead of the document. Every node id of the document
    /// before goes stale (p1-contract §2), and every subscription to its
    /// nodes ends.
    pub fn load_html(&mut self, html: &str) {
        self.engine.load_html(html);
        self.forget_gone_nodes();
    }

    /// The document node.
    pub fn root(&self) -> Node {
        self.outward(self.engine.root())
            .expect("the document node has an id")
    }

    /// The first element inside `scope` (the whole document for `None`)
    /// matching the CSS selector list `selector`, in document order.
    pub fn query(&self, scope: Option<Node>, selector: &str) -> Result<Option<Node>, Status> {
        let scope = scope.map(|node| self.inward(node));
        let found = self.engine.query(scope, selector)?;
        Ok(found.and_then(|id| self.outward(id)))
    }

    /// Every element inside `scope` (the whole document for `None`)
    /// matching the CSS selector list `selector`, in document order.
    pub fn query_all(&self, scope: Option<Node>, selector: &str) -> Result<Vec<Node>, Status> {
        let scope = scope.map(|node| self.inward(node));
        let found = self.engine.query_all(scope, selector)?;
        Ok(found
            .into_iter()
            .filter_map(|id| self.outward(id))
            .collect())
    }

    /// Apply `mutations` in order (M4 plan, decision 1): one call for many
    /// changes, as the bindings want. A mutation may name the node an
    /// earlier one in the batch created ([`Ref::New`]). Returns the nodes
    /// the batch created, by position (`None` for the mutations that create
    /// nothing). The first that fails stops the batch, and the error says
    /// which it was; the ones before it stay applied: transactions come
    /// with M5's journal.
    pub fn apply(&mut self, mutations: &[Mutation]) -> Result<Vec<Option<Node>>, BatchError> {
        let mut created: Vec<Option<Node>> = Vec::with_capacity(mutations.len());
        for (index, mutation) in mutations.iter().enumerate() {
            let fail = |status| BatchError { index, status };
            let made = self.apply_one(mutation, &created).map_err(fail)?;
            created.push(made);
        }
        Ok(created)
    }

    fn apply_one(
        &mut self,
        mutation: &Mutation,
        created: &[Option<Node>],
    ) -> Result<Option<Node>, Status> {
        let node = |reference: &Ref| match *reference {
            Ref::Node(node) => Ok(node),
            Ref::New(at) => created
                .get(at)
                .copied()
                .flatten()
                .ok_or(Status::InvalidArgument),
        };
        match mutation {
            Mutation::CreateElement(tag) => return self.create_element(tag).map(Some),
            Mutation::CreateText(text) => return Ok(Some(self.create_text(text))),
            Mutation::Append { parent, child } => self.append(node(parent)?, node(child)?)?,
            Mutation::InsertBefore {
                parent,
                child,
                before,
            } => {
                let before = before.as_ref().map(node).transpose()?;
                self.insert_before(node(parent)?, node(child)?, before)?;
            }
            Mutation::Remove(target) => self.remove(node(target)?)?,
            Mutation::SetText(target, text) => self.set_text(node(target)?, text)?,
            Mutation::SetAttr(target, name, value) => self.set_attr(node(target)?, name, value)?,
            // Removing an attribute that is not there is no error, as in DOM.
            Mutation::RemoveAttr(target, name) => {
                self.remove_attr(node(target)?, name)?;
            }
            Mutation::AddClass(target, class) => self.add_class(node(target)?, class)?,
            Mutation::RemoveClass(target, class) => self.remove_class(node(target)?, class)?,
        }
        Ok(None)
    }

    /// Set `node`'s text as the DOM's `textContent` does: an element's
    /// children are replaced by one text node, and subscriptions to them
    /// end. The next frame shows it.
    pub fn set_text(&mut self, node: Node, text: &str) -> Result<(), Status> {
        let id = self.inward(node);
        self.engine.set_text(id, text)?;
        self.forget_gone_nodes();
        Ok(())
    }

    /// A new element named `tag`, not in the document yet: insert it with
    /// [`Context::append`] or [`Context::insert_before`], or let it go with
    /// [`Context::remove`]. HTML lowercases the name; `InvalidArgument` for
    /// one that is not a name.
    pub fn create_element(&mut self, tag: &str) -> Result<Node, Status> {
        let id = self.engine.create_element(tag)?;
        Ok(self.outward(id).expect("a new node has an id"))
    }

    /// A new text node, not in the document yet.
    pub fn create_text(&mut self, text: &str) -> Node {
        let id = self.engine.create_text(text);
        self.outward(id).expect("a new node has an id")
    }

    /// Make `child` the last child of `parent`, moving it from wherever it
    /// was. `InvalidArgument` where DOM refuses it: a node into itself or
    /// its descendant, the document node, text into the document.
    pub fn append(&mut self, parent: Node, child: Node) -> Result<(), Status> {
        let (parent, child) = (self.inward(parent), self.inward(child));
        Ok(self.engine.insert(parent, child, None)?)
    }

    /// Insert `child` into `parent` before `before`, a child of `parent`,
    /// or last for `None` (DOM's `insertBefore`).
    pub fn insert_before(
        &mut self,
        parent: Node,
        child: Node,
        before: Option<Node>,
    ) -> Result<(), Status> {
        let (parent, child) = (self.inward(parent), self.inward(child));
        let before = before.map(|node| self.inward(node));
        Ok(self.engine.insert(parent, child, before)?)
    }

    /// Remove `node` and everything in it: their ids go stale, and their
    /// subscriptions end. `InvalidArgument` for the document node.
    pub fn remove(&mut self, node: Node) -> Result<(), Status> {
        self.engine.remove(self.inward(node))?;
        self.forget_gone_nodes();
        Ok(())
    }

    /// Set attribute `name` of element `node` to `value`; HTML lowercases
    /// the name. The next frame restyles with it (`class`, `id`, `style`,
    /// attribute selectors).
    pub fn set_attr(&mut self, node: Node, name: &str, value: &str) -> Result<(), Status> {
        Ok(self.engine.set_attr(self.inward(node), name, value)?)
    }

    /// Remove attribute `name` of element `node`; whether it had one.
    pub fn remove_attr(&mut self, node: Node, name: &str) -> Result<bool, Status> {
        Ok(self.engine.remove_attr(self.inward(node), name)?)
    }

    /// Attribute `name` of `node`; `None` without one, or for a node that is
    /// not an element.
    pub fn attr(&self, node: Node, name: &str) -> Result<Option<String>, Status> {
        Ok(self.engine.attr(self.inward(node), name)?)
    }

    /// Whether element `node`'s `class` holds `class`, as DOM's `classList`
    /// reads it: classes are separated by ASCII white space.
    pub fn has_class(&self, node: Node, class: &str) -> Result<bool, Status> {
        let classes = self.attr(node, "class")?.unwrap_or_default();
        Ok(classes.split_ascii_whitespace().any(|c| c == class))
    }

    /// Add `class` to element `node`'s classes, once. `InvalidArgument` for
    /// an empty class or one with white space in it.
    pub fn add_class(&mut self, node: Node, class: &str) -> Result<(), Status> {
        let mut classes = self.classes(node, class)?;
        if !classes.iter().any(|c| c == class) {
            classes.push(class.to_owned());
            self.set_attr(node, "class", &classes.join(" "))?;
        }
        Ok(())
    }

    /// Remove `class` from element `node`'s classes.
    pub fn remove_class(&mut self, node: Node, class: &str) -> Result<(), Status> {
        let mut classes = self.classes(node, class)?;
        let before = classes.len();
        classes.retain(|c| c != class);
        if classes.len() != before {
            self.set_attr(node, "class", &classes.join(" "))?;
        }
        Ok(())
    }

    /// `node`'s classes, after checking `class` is one class.
    fn classes(&self, node: Node, class: &str) -> Result<Vec<String>, Status> {
        if class.is_empty() || class.contains(|c: char| c.is_ascii_whitespace()) {
            return Err(Status::InvalidArgument);
        }
        Ok(self
            .attr(node, "class")?
            .unwrap_or_default()
            .split_ascii_whitespace()
            .map(str::to_owned)
            .collect())
    }

    /// `node`'s text as the DOM's `textContent` reads it.
    pub fn text(&self, node: Node) -> Result<String, Status> {
        Ok(self.engine.text(self.inward(node))?)
    }

    /// `node`'s parent; `None` for the document node.
    pub fn parent(&self, node: Node) -> Result<Option<Node>, Status> {
        let parent = self.engine.parent(self.inward(node))?;
        Ok(parent.and_then(|id| self.outward(id)))
    }

    /// `node`'s child at `index`, in document order; `None` past the last.
    pub fn child_at(&self, node: Node, index: usize) -> Result<Option<Node>, Status> {
        let child = self.engine.child_at(self.inward(node), index)?;
        Ok(child.and_then(|id| self.outward(id)))
    }

    /// How many children `node` has.
    pub fn child_count(&self, node: Node) -> Result<usize, Status> {
        Ok(self.engine.child_count(self.inward(node))?)
    }

    /// What `node` is: the document, an element, text, a comment.
    pub fn kind(&self, node: Node) -> Result<NodeKind, Status> {
        Ok(self.engine.kind(self.inward(node))?)
    }

    /// An element's tag name, lowercase as HTML parses it; `None` for a node
    /// that is not an element.
    pub fn tag(&self, node: Node) -> Result<Option<String>, Status> {
        Ok(self.engine.tag(self.inward(node))?)
    }

    /// An element's attributes, as written, in order.
    pub fn attributes(&self, node: Node) -> Result<Vec<(String, String)>, Status> {
        Ok(self.engine.attributes(self.inward(node))?)
    }

    /// `node`'s box in the last frame (p1-contract §8.1): the border box in
    /// CSS pixels relative to the viewport, and its margin, border and
    /// padding. `NotFound` for a node without one: text, an inline
    /// element, one not displayed, or any node before the first frame.
    pub fn node_box(&self, node: Node) -> Result<BoxModel, Status> {
        self.engine
            .node_box(self.inward(node))?
            .ok_or(Status::NotFound)
    }

    /// `node`'s computed style in the last frame, as `name: value;` lines,
    /// one per property Erk uses, in the order of their names. `NotFound`
    /// for a node without a style.
    pub fn computed_style(&self, node: Node) -> Result<String, Status> {
        self.engine
            .computed_style(self.inward(node))?
            .ok_or(Status::NotFound)
    }

    /// The topmost node at `x`, `y` (CSS pixels) in the last frame, as a
    /// click there would find it (p1-contract §8.1, `erk_inspect_at`).
    pub fn inspect_at(&self, x: f32, y: f32) -> Option<Node> {
        self.engine.inspect_at(x, y).and_then(|id| self.outward(id))
    }

    /// What the last frame did, counted: elements styled, boxes laid out,
    /// paragraphs shaped, display list items (M5.0).
    pub fn frame_stats(&self) -> FrameStats {
        self.engine.stats()
    }

    /// Keep each frame to check it against the oracle with
    /// [`Context::verify_frame`]. For tests and fuzzing: it costs a copy of
    /// every frame.
    #[doc(hidden)]
    pub fn set_verifying(&mut self, verifying: bool) {
        self.engine.set_verifying(verifying);
    }

    /// Check the last frame against the document recomputed from nothing
    /// (M5 plan, decision 9); the error says where they part.
    #[doc(hidden)]
    pub fn verify_frame(&mut self) -> Result<(), String> {
        self.engine.verify()
    }

    /// Draw the developer tools' highlight over `node`'s boxes from the next
    /// frame on, or none. It is drawn over the page, never added to the
    /// document.
    pub fn highlight(&mut self, node: Option<Node>) -> Result<(), Status> {
        let id = match node {
            Some(node) => {
                let id = self.inward(node);
                if !self.engine.contains(id) {
                    return Err(Status::StaleNode);
                }
                Some(id)
            }
            None => None,
        };
        self.engine.highlight(id);
        Ok(())
    }

    /// Call `callback` when an event of `kind` reaches `node` at its target
    /// or on its way back up (the bubble phase). The callback is dropped,
    /// once, when the subscription ends: by [`Context::off`], when the node
    /// leaves the document, or with the app (p1-contract §5).
    pub fn on(
        &mut self,
        node: Node,
        kind: EventKind,
        callback: impl FnMut(&mut Context, &Event) + 'static,
    ) -> Result<Subscription, Status> {
        self.subscribe(node, kind, false, Box::new(callback))
    }

    /// Like [`Context::on`], in the capture phase: on the way down from the
    /// root, before the target, and at the target before the bubble
    /// subscriptions there.
    pub fn on_capture(
        &mut self,
        node: Node,
        kind: EventKind,
        callback: impl FnMut(&mut Context, &Event) + 'static,
    ) -> Result<Subscription, Status> {
        self.subscribe(node, kind, true, Box::new(callback))
    }

    fn subscribe(
        &mut self,
        node: Node,
        kind: EventKind,
        capture: bool,
        callback: events::Callback,
    ) -> Result<Subscription, Status> {
        let id = self.inward(node);
        if !self.engine.contains(id) {
            return Err(Status::StaleNode);
        }
        Ok(self.listeners.add(Listener {
            node: id,
            kind,
            capture,
            callback: Some(callback),
        }))
    }

    /// End `subscription`; its callback is dropped now or, when it is the
    /// one running, as soon as it returns. `NotFound` if it has ended
    /// already.
    pub fn off(&mut self, subscription: Subscription) -> Result<(), Status> {
        self.listeners
            .remove(subscription)
            .map(drop)
            .ok_or(Status::NotFound)
    }

    /// Inside an event callback: the event goes no further than the node it
    /// is at; the other subscriptions there still run.
    pub fn stop_propagation(&mut self) {
        self.stopped = true;
    }

    /// A handle any thread may use to reach this app.
    pub fn handle(&self) -> AppHandle {
        AppHandle {
            inbox: self.entry.clone(),
        }
    }

    /// The answer to resource request `id`, for the provider.
    pub(crate) fn responder(&self, id: u64) -> Responder {
        Responder {
            id,
            inbox: Some(self.entry.clone()),
        }
    }

    /// From now on, a message from another thread also calls `wake`.
    pub(crate) fn wake_with(&self, wake: impl Fn() + Send + Sync + 'static) {
        self.entry.waker.set(wake);
    }

    /// Send `event` through its phases (p1-contract §5): down the path from
    /// the root element in the capture phase, at the target, and back up in
    /// the bubble phase for the kinds that bubble. A panic in a callback
    /// reaches the caller once the subscription is back in place.
    pub(crate) fn dispatch(&mut self, event: erk_renderer::Event) {
        let Some(&target) = event.path.first() else {
            return;
        };
        let ancestors = &event.path[1..];
        self.stopped = false;
        for &node in ancestors.iter().rev() {
            if self.call(node, &event, true, Phase::Capture) {
                return;
            }
        }
        let capture_stopped = self.call(target, &event, true, Phase::Target);
        if self.call(target, &event, false, Phase::Target) || capture_stopped {
            return;
        }
        if events::bubbles(event.kind) {
            for &node in ancestors {
                if self.call(node, &event, false, Phase::Bubble) {
                    return;
                }
            }
        }
    }

    /// Call the subscriptions at `node`; whether propagation has stopped.
    fn call(
        &mut self,
        node: u64,
        event: &erk_renderer::Event,
        capture: bool,
        phase: Phase,
    ) -> bool {
        let (Some(target), Some(current_target)) = (self.outward(event.target), self.outward(node))
        else {
            return self.stopped;
        };
        let event = Event {
            kind: event.kind,
            phase,
            target,
            current_target,
            x: event.x,
            y: event.y,
            modifiers: event.modifiers,
            key: event.key.clone(),
        };
        for id in self.listeners.at(node, event.kind, capture) {
            // Ended by an earlier callback: not called (DOM's removed flag).
            let Some(mut callback) = self.listeners.take(id) else {
                continue;
            };
            let result = catch_unwind(AssertUnwindSafe(|| callback(self, &event)));
            self.listeners.put_back(id, callback);
            if let Err(panic) = result {
                resume_unwind(panic);
            }
        }
        self.stopped
    }

    /// Take what other threads sent off the inbox; whether there was any.
    pub(crate) fn collect(&mut self) -> bool {
        self.queued.extend(self.inbox.try_iter());
        !self.queued.is_empty()
    }

    /// Handle what other threads sent: posted work and resource answers.
    pub(crate) fn drain(&mut self) {
        self.collect();
        while let Some(message) = self.queued.pop_front() {
            match message {
                Message::Post(work) => work(self),
                Message::Resource(response) => self.engine.complete_resource(&response),
                Message::Missing(id) => self.engine.resource_missing(id),
            }
        }
    }

    /// End the subscriptions to nodes no longer in the document.
    fn forget_gone_nodes(&mut self) {
        let engine = &self.engine;
        self.listeners.retain_nodes(|node| engine.contains(node));
    }

    fn inward(&self, node: Node) -> u64 {
        ids::inward(node, self.key)
    }

    pub(crate) fn outward(&self, id: u64) -> Option<Node> {
        ids::outward(id, self.key)
    }
}
