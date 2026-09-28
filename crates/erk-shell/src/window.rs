//! The window: winit for events, softbuffer to put the renderer's frames on
//! screen.
//!
//! Frames arrive on the renderer's channel, which the event loop cannot wait
//! on, so a small forwarding thread turns each one into a winit user event.
//! The renderer never sees winit.

use std::num::NonZeroU32;
use std::path::Path;
use std::rc::Rc;
use std::sync::mpsc::Sender;

use erk_renderer::{Frame, FromRenderer, ToRenderer};
use softbuffer::{Context, Surface};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

/// Initial window size in logical pixels, the same as the golden images.
const INITIAL_SIZE: LogicalSize<f64> = LogicalSize::new(800.0, 600.0);

enum UserEvent {
    Frame(Frame),
}

/// Open `page` (already read as `html`) in a window and run until it closes.
pub(crate) fn run(page: &Path, html: String) -> Result<(), winit::error::EventLoopError> {
    let event_loop = EventLoop::<UserEvent>::with_user_event().build()?;

    let (to_renderer, from_renderer, renderer) = erk_renderer::spawn();
    let proxy = event_loop.create_proxy();
    let forwarder = std::thread::Builder::new()
        .name("erk-frames".to_owned())
        .spawn(move || {
            for FromRenderer::Frame(frame) in from_renderer {
                if proxy.send_event(UserEvent::Frame(frame)).is_err() {
                    break; // the event loop has exited
                }
            }
        })
        .expect("the frame forwarding thread starts");

    // A send only fails if the renderer is gone, which the forwarder and
    // join below report.
    let _ = to_renderer.send(ToRenderer::Load { html });
    let title = format!(
        "Erk — {}",
        page.file_name().map_or_else(
            || page.display().to_string(),
            |name| name.to_string_lossy().into_owned()
        )
    );
    let mut app = App {
        title,
        to_renderer,
        window: None,
        frame: None,
    };
    let result = event_loop.run_app(&mut app);

    let _ = app.to_renderer.send(ToRenderer::Shutdown);
    let _ = renderer.join();
    let _ = forwarder.join();
    result
}

struct WindowState {
    window: Rc<Window>,
    surface: Surface<Rc<Window>, Rc<Window>>,
}

struct App {
    title: String,
    to_renderer: Sender<ToRenderer>,
    window: Option<WindowState>,
    /// The latest frame from the renderer, kept for redraws.
    frame: Option<Frame>,
}

impl App {
    fn request_frame(&self, size: PhysicalSize<u32>) {
        // M0: one CSS pixel per device pixel, so the viewport is the window's
        // physical size. Device scale arrives with the GPU path (M2).
        let clamp = |v: u32| u16::try_from(v).unwrap_or(u16::MAX);
        let _ = self.to_renderer.send(ToRenderer::Resize {
            width: clamp(size.width),
            height: clamp(size.height),
        });
    }

    fn redraw(&mut self) {
        let Some(state) = &mut self.window else {
            return;
        };
        let size = state.window.inner_size();
        let (Some(width), Some(height)) =
            (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
        else {
            return; // minimised
        };
        if state.surface.resize(width, height).is_err() {
            return;
        }
        let Ok(mut buffer) = state.surface.buffer_mut() else {
            return;
        };
        let (width, height) = (size.width as usize, size.height as usize);
        // White until the renderer has caught up with the window's size.
        buffer.fill(0x00ff_ffff);
        if let Some(frame) = &self.frame {
            blit(frame, &mut buffer, width, height);
        }
        let _ = buffer.present();
    }
}

/// Copy the overlapping part of `frame` into a softbuffer buffer of
/// `width` × `height`. Frames are opaque, so premultiplied RGBA is also
/// straight RGBA, and softbuffer wants `0x00RRGGBB`.
fn blit(frame: &Frame, buffer: &mut [u32], width: usize, height: usize) {
    let frame_width = usize::from(frame.width());
    let rows = height.min(usize::from(frame.height()));
    let columns = width.min(frame_width);
    let pixels = frame.rgba().as_chunks::<4>().0;
    for y in 0..rows {
        let source = &pixels[y * frame_width..][..columns];
        let target = &mut buffer[y * width..][..columns];
        for (out, [r, g, b, _]) in target.iter_mut().zip(source) {
            *out = u32::from(*r) << 16 | u32::from(*g) << 8 | u32::from(*b);
        }
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title(&self.title)
            .with_inner_size(INITIAL_SIZE);
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Rc::new(window),
            Err(error) => {
                eprintln!("erk: cannot create a window: {error}");
                event_loop.exit();
                return;
            }
        };
        let surface =
            Context::new(window.clone()).and_then(|context| Surface::new(&context, window.clone()));
        let surface = match surface {
            Ok(surface) => surface,
            Err(error) => {
                eprintln!("erk: cannot draw into the window: {error}");
                event_loop.exit();
                return;
            }
        };
        self.request_frame(window.inner_size());
        self.window = Some(WindowState { window, surface });
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                self.request_frame(size);
                if let Some(state) = &self.window {
                    state.window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => self.redraw(),
            _ => {}
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Frame(frame) => {
                self.frame = Some(frame);
                if let Some(state) = &self.window {
                    state.window.request_redraw();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blit_converts_to_xrgb_and_clips_to_the_buffer() {
        let frame = erk_renderer::render_html(r#"<html style="background: #123456"></html>"#, 4, 4);
        let mut buffer = vec![0u32; 3 * 2];
        blit(&frame, &mut buffer, 3, 2);
        assert!(
            buffer.iter().all(|&pixel| pixel == 0x0012_3456),
            "{buffer:x?}"
        );
    }

    #[test]
    fn blit_leaves_the_rest_of_a_larger_buffer_alone() {
        let frame = erk_renderer::render_html(r#"<html style="background: #123456"></html>"#, 2, 1);
        let mut buffer = vec![0x00ff_ffffu32; 3 * 2];
        blit(&frame, &mut buffer, 3, 2);
        assert_eq!(
            buffer,
            [
                0x0012_3456,
                0x0012_3456,
                0x00ff_ffff,
                0x00ff_ffff,
                0x00ff_ffff,
                0x00ff_ffff
            ]
        );
    }
}
