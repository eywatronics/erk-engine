//! The two things any thread may do with an app (p1-contract §4): post work
//! to its UI thread, and answer a resource request. Both go through a
//! channel the app drains on its UI thread before the next frame.

use std::sync::mpsc::Sender;

use crate::{Context, Status};

pub(crate) type Posted = Box<dyn FnOnce(&mut Context) + Send>;

pub(crate) enum Message {
    Post(Posted),
    Resource(erk_renderer::ResourceResponse),
    Missing(u64),
}

/// A handle any thread may hold: it reaches the app's UI thread.
#[derive(Clone)]
pub struct AppHandle {
    pub(crate) to: Sender<Message>,
}

impl AppHandle {
    /// Run `work` on the app's UI thread before its next frame: the one way
    /// back to the UI from background work. `NotFound` when the app is gone;
    /// `work` is then dropped unrun.
    pub fn post(&self, work: impl FnOnce(&mut Context) + Send + 'static) -> Result<(), Status> {
        self.to
            .send(Message::Post(Box::new(work)))
            .map_err(|_| Status::NotFound)
    }
}

/// The answer to one resource request, given to the host's resource
/// provider. It may answer at once, or move this to another thread and
/// answer later. Erk copies the bytes. Dropped unanswered, the resource is
/// missing: the page renders without it.
pub struct Responder {
    pub(crate) id: u64,
    pub(crate) to: Option<Sender<Message>>,
}

impl Responder {
    /// The resource's bytes. An empty `mime` lets Erk tell the type from the
    /// bytes and the request's kind; a response that is not what was asked
    /// for is refused and logged.
    pub fn respond(mut self, mime: &str, data: Vec<u8>) {
        if let Some(to) = self.to.take() {
            let _ = to.send(Message::Resource(erk_renderer::ResourceResponse {
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
        if let Some(to) = self.to.take() {
            let _ = to.send(Message::Missing(self.id));
        }
    }
}
