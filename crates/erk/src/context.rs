//! What a callback may do: everything an app does but run its loop or end
//! (p1-contract §5). An [`crate::App`] dereferences to its context, so the
//! host and its callbacks use one API; a callback gets `&mut Context` and
//! so cannot tick, run or drop the app it is called from.

use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::mpsc::{Receiver, Sender};

use erk_renderer::Engine;

use crate::Status;
use crate::events::{self, Event, EventKind, Listener, Listeners, Phase, Subscription};
use crate::handle::{AppHandle, Message};
use crate::ids::{self, Node};

pub struct Context {
    pub(crate) engine: Engine,
    /// The key this app's node ids are mixed with (p1-contract §2).
    key: u64,
    listeners: Listeners,
    /// Set by [`Context::stop_propagation`] during a dispatch.
    stopped: bool,
    /// What other threads sent, and the sender handles copy.
    to: Sender<Message>,
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
            to,
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
            to: self.to.clone(),
        }
    }

    pub(crate) fn sender(&self) -> Sender<Message> {
        self.to.clone()
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
