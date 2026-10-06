//! What a callback may do: everything an app does but run its loop or end
//! (p1-contract §5). An [`crate::App`] dereferences to its context, so the
//! host and its callbacks use one API; a callback gets `&mut Context` and
//! so cannot tick, run or drop the app it is called from.

use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::mpsc::Receiver;

use erk_renderer::{BoxModel, Engine, NodeKind};

use crate::Status;
use crate::events::{self, Event, EventKind, Listener, Listeners, Phase, Subscription};
use crate::handle::{AppHandle, Inbox, Message, Responder, Waker};
use crate::ids::{self, Node};

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

    /// Set `node`'s text as the DOM's `textContent` does: an element's
    /// children are replaced by one text node, and subscriptions to them
    /// end. The next frame shows it.
    pub fn set_text(&mut self, node: Node, text: &str) -> Result<(), Status> {
        let id = self.inward(node);
        self.engine.set_text(id, text)?;
        self.forget_gone_nodes();
        Ok(())
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
