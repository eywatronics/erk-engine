//! The two things any thread may do with an app (p1-contract §4): post work
//! to its UI thread, and answer a resource request. Both go through a
//! channel the app drains on its UI thread before the next frame, and wake
//! its event loop when it has one.

use std::sync::mpsc::Sender;
use std::sync::{Arc, OnceLock};

use crate::{Context, Status};

pub(crate) type Posted = Box<dyn FnOnce(&mut Context) + Send>;

pub(crate) enum Message {
    Post(Posted),
    Resource(erk_renderer::ResourceResponse),
    Missing(u64),
}

/// Wakes the app's event loop when a message arrives. A windowless app has
/// no loop to wake: its host ticks it.
#[derive(Clone, Default)]
pub(crate) struct Waker(Arc<OnceLock<Box<dyn Fn() + Send + Sync>>>);

impl Waker {
    /// From now on, wake with `wake`.
    pub(crate) fn set(&self, wake: impl Fn() + Send + Sync + 'static) {
        let _ = self.0.set(Box::new(wake));
    }

    fn wake(&self) {
        if let Some(wake) = self.0.get() {
            wake();
        }
    }
}

/// The way into an app: its channel and its waker.
#[derive(Clone)]
pub(crate) struct Inbox {
    pub(crate) to: Sender<Message>,
    pub(crate) waker: Waker,
}

impl Inbox {
    fn send(&self, message: Message) -> Result<(), Status> {
        self.to.send(message).map_err(|_| Status::NotFound)?;
        self.waker.wake();
        Ok(())
    }
}

/// A handle any thread may hold: it reaches the app's UI thread.
#[derive(Clone)]
pub struct AppHandle {
    pub(crate) inbox: Inbox,
}

impl AppHandle {
    /// Run `work` on the app's UI thread before its next frame: the one way
    /// back to the UI from background work. `NotFound` when the app is gone;
    /// `work` is then dropped unrun.
    pub fn post(&self, work: impl FnOnce(&mut Context) + Send + 'static) -> Result<(), Status> {
        self.inbox.send(Message::Post(Box::new(work)))
    }
}

/// The answer to one resource request, given to the host's resource
/// provider. It may answer at once, or move this to another thread and
/// answer later. Erk copies the bytes. Dropped unanswered, the resource is
/// missing: the page renders without it.
pub struct Responder {
    pub(crate) id: u64,
    pub(crate) inbox: Option<Inbox>,
}

impl Responder {
    /// The resource's bytes. An empty `mime` lets Erk tell the type from the
    /// bytes and the request's kind; a response that is not what was asked
    /// for is refused and logged.
    pub fn respond(mut self, mime: &str, data: Vec<u8>) {
        if let Some(inbox) = self.inbox.take() {
            let _ = inbox.send(Message::Resource(erk_renderer::ResourceResponse {
                id: self.id,
                mime: mime.to_owned(),
                data,
            }));
        }
    }

    /// There is no such resource.
    pub fn missing(self) {
        drop(self);
    }
}

impl Drop for Responder {
    fn drop(&mut self) {
        if let Some(inbox) = self.inbox.take() {
            let _ = inbox.send(Message::Missing(self.id));
        }
    }
}
