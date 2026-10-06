//! The window: winit for events; the renderer draws into it on the GPU, or
//! sends frames that softbuffer puts on screen.
//!
//! The renderer starts once the window exists, on the window: its GPU path
//! draws into the window's surface (M2.5). Frames, when it falls back to the
//! CPU, arrive on its channel, which the event loop cannot wait on, so a
//! small forwarding thread turns each message into a winit user event. The
//! renderer never sees winit: it takes the window as a raw window handle.

use std::num::NonZeroU32;
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};
use std::thread::JoinHandle;

use erk_renderer::{
    Cursor, FontCatalog, Frame, FromRenderer, Key, KeyInput, KeyState, Modifiers, PointerButton,
    PointerInput, PointerKind, Raster, ToRenderer,
};
use softbuffer::{Context, Surface};

use crate::counter::Counter;
use crate::fonts::SystemFonts;
use crate::resources::Provider;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::keyboard::{Key as WinitKey, NamedKey};
use winit::window::{CursorIcon, Window, WindowId};

/// Initial window size in logical pixels, the same as the golden images.
const INITIAL_SIZE: LogicalSize<f64> = LogicalSize::new(800.0, 600.0);

enum UserEvent {
    Frame(Frame),
    Cursor(Cursor),
    Raster(Raster),
    /// The renderer's channel closed while the window was open: the
    /// renderer has stopped, and the window would only show a stale frame.
    RendererGone,
}

/// What the renderer starts with, once the window exists.
struct Startup {
    html: String,
    fonts: FontCatalog,
    provider: Provider,
    /// Whether to try the GPU path.
    gpu: bool,
}

/// The running renderer and the thread that forwards what it says.
struct Running {
    to: Sender<ToRenderer>,
    thread: JoinHandle<()>,
    forwarder: JoinHandle<()>,
}

/// Open `page` (already read as `html`) in a window and run until it closes.
pub(crate) fn run(page: &Path, html: String, gpu: bool) -> Result<(), String> {
    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .map_err(|e| e.to_string())?;
    // The system's fonts: the catalogue goes to the renderer before the
    // page, the files are read when it asks for them.
    let fonts = Arc::new(SystemFonts::scan());
    let catalogue = fonts.catalogue().clone();
    let provider = Provider::for_page(page).with_fonts(fonts);
    let title = format!(
        "Erk — {}",
        page.file_name().map_or_else(
            || page.display().to_string(),
            |name| name.to_string_lossy().into_owned()
        )
    );
    let mut app = App {
        title,
        proxy: event_loop.create_proxy(),
        startup: Some(Startup {
            html,
            fonts: catalogue,
            provider,
            gpu,
        }),
        renderer: None,
        window: None,
        frame: None,
        pointer: (0.0, 0.0),
        modifiers: Modifiers::default(),
    };
    let result = event_loop.run_app(&mut app);

    let renderer = match app.renderer.take() {
        Some(running) => {
            let _ = running.to.send(ToRenderer::Shutdown);
            let joined = running.thread.join();
            let _ = running.forwarder.join();
            joined
        }
        None => Ok(()),
    };
    finish(result, renderer)
}

/// Forward what the renderer says to the event loop, answering its
/// resource requests and running the demo host on the way: the host reads
/// files, the renderer never does. Returns the thread and the questions
/// the demo host asks the page, to send after it.
fn forward(
    from: Receiver<FromRenderer>,
    answers: Sender<ToRenderer>,
    provider: Provider,
    proxy: EventLoopProxy<UserEvent>,
) -> (JoinHandle<()>, Vec<ToRenderer>) {
    // The demo host: the counter, on pages that have one.
    let (mut counter, questions) = Counter::start();
    let thread = std::thread::Builder::new()
        .name("erk-frames".to_owned())
        .spawn(move || {
            for message in from {
                for answer in counter.on(&message) {
                    let _ = answers.send(answer);
                }
                let event = match message {
                    FromRenderer::Frame(frame) => UserEvent::Frame(frame),
                    FromRenderer::Cursor(cursor) => UserEvent::Cursor(cursor),
                    FromRenderer::Raster(raster) => UserEvent::Raster(raster),
                    FromRenderer::Resources(requests) => {
                        for request in &requests {
                            let _ = answers.send(provider.answer(request));
                        }
                        continue;
                    }
                    // Drawn into the window already; events, answers and
                    // changes are the demo host's.
                    FromRenderer::Presented { .. }
                    | FromRenderer::Event(_)
                    | FromRenderer::Inspected { .. }
                    | FromRenderer::QueryResult { .. }
                    | FromRenderer::Done { .. } => continue,
                };
                if proxy.send_event(event).is_err() {
                    return; // the event loop has exited
                }
            }
            // Fails harmlessly when the event loop has already exited, as it
            // has after a normal shutdown.
            let _ = proxy.send_event(UserEvent::RendererGone);
        })
        .expect("the frame forwarding thread starts");
    (thread, questions)
}

/// The window's outcome. A renderer that panicked is an error even though
/// the event loop, which it closed, ended cleanly.
fn finish(
    event_loop: Result<(), winit::error::EventLoopError>,
    renderer: std::thread::Result<()>,
) -> Result<(), String> {
    if renderer.is_err() {
        return Err("the renderer thread panicked".to_owned());
    }
    event_loop.map_err(|e| e.to_string())
}

struct WindowState {
    window: Arc<Window>,
    /// softbuffer's surface, made when the first CPU frame arrives: on the
    /// GPU path the renderer draws into the window itself.
    surface: Option<Surface<Arc<Window>, Arc<Window>>>,
}

struct App {
    title: String,
    proxy: EventLoopProxy<UserEvent>,
    /// Until the window exists.
    startup: Option<Startup>,
    renderer: Option<Running>,
    window: Option<WindowState>,
    /// The latest frame from the renderer, kept for redraws.
    frame: Option<Frame>,
    /// Where the pointer is, in CSS pixels, and the modifier keys held:
    /// a button event carries neither.
    pointer: (f32, f32),
    modifiers: Modifiers,
}

impl App {
    fn send(&self, message: ToRenderer) {
        // A send only fails if the renderer is gone, which the forwarder
        // and `finish` report.
        if let Some(running) = &self.renderer {
            let _ = running.to.send(message);
        }
    }

    /// Start the renderer on `window`.
    fn start_renderer(&mut self, window: &Arc<Window>) {
        let Some(startup) = self.startup.take() else {
            return;
        };
        let (to, from, thread) = if startup.gpu {
            erk_renderer::spawn_on_window(window.clone())
        } else {
            erk_renderer::spawn()
        };
        let (forwarder, questions) =
            forward(from, to.clone(), startup.provider, self.proxy.clone());
        self.renderer = Some(Running {
            to,
            thread,
            forwarder,
        });
        self.send(ToRenderer::Fonts(startup.fonts));
        self.send(ToRenderer::Load { html: startup.html });
        for question in questions {
            self.send(question);
        }
    }

    /// The renderer paints at the window's scale (device pixels per CSS
    /// pixel): a page laid out for 800 CSS pixels fills an 800-point window
    /// on any screen, sharply.
    fn send_scale(&self, window: &Window) {
        self.send(ToRenderer::Scale {
            factor: window.scale_factor() as f32,
        });
    }

    fn send_pointer(&self, kind: PointerKind, button: PointerButton) {
        self.send(ToRenderer::Pointer(PointerInput {
            kind,
            x: self.pointer.0,
            y: self.pointer.1,
            button,
            modifiers: self.modifiers,
        }));
    }

    /// The viewport is the window's size in device pixels.
    fn request_frame(&self, size: PhysicalSize<u32>) {
        let clamp = |v: u32| u16::try_from(v).unwrap_or(u16::MAX);
        self.send(ToRenderer::Resize {
            width: clamp(size.width),
            height: clamp(size.height),
        });
    }

    fn redraw(&mut self) {
        // Only frames the CPU painted are this window's to show.
        let (Some(state), Some(frame)) = (&mut self.window, &self.frame) else {
            return;
        };
        if state.surface.is_none() {
            let surface = Context::new(state.window.clone())
                .and_then(|context| Surface::new(&context, state.window.clone()));
            match surface {
                Ok(surface) => state.surface = Some(surface),
                Err(error) => {
                    eprintln!("erk: cannot draw into the window: {error}");
                    return;
                }
            }
        }
        let Some(surface) = state.surface.as_mut() else {
            return;
        };
        let size = state.window.inner_size();
        let (Some(width), Some(height)) =
            (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
        else {
            return; // minimised
        };
        if surface.resize(width, height).is_err() {
            return;
        }
        let Ok(mut buffer) = surface.buffer_mut() else {
            return;
        };
        let (width, height) = (size.width as usize, size.height as usize);
        // White until the renderer has caught up with the window's size.
        buffer.fill(0x00ff_ffff);
        blit(frame, &mut buffer, width, height);
        let _ = buffer.present();
    }
}

/// The renderer's name for a key winit reports.
fn key(logical: &WinitKey) -> Key {
    match logical {
        WinitKey::Named(NamedKey::Tab) => Key::Tab,
        WinitKey::Named(NamedKey::Enter) => Key::Enter,
        WinitKey::Named(NamedKey::Space) => Key::Space,
        WinitKey::Named(NamedKey::Escape) => Key::Escape,
        // Some platforms report the space bar as the character it types.
        WinitKey::Character(text) if text.as_str() == " " => Key::Space,
        WinitKey::Character(text) => Key::Character(text.to_string()),
        _ => Key::Other,
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
            Ok(window) => Arc::new(window),
            Err(error) => {
                eprintln!("erk: cannot create a window: {error}");
                event_loop.exit();
                return;
            }
        };
        self.start_renderer(&window);
        self.send_scale(&window);
        self.request_frame(window.inner_size());
        self.window = Some(WindowState {
            window,
            surface: None,
        });
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
            // Moved to a screen of another scale; a Resized follows.
            WindowEvent::ScaleFactorChanged { .. } => {
                if let Some(state) = &self.window {
                    self.send_scale(&state.window);
                }
            }
            WindowEvent::RedrawRequested => self.redraw(),
            WindowEvent::CursorMoved { position, .. } => {
                let scale = self
                    .window
                    .as_ref()
                    .map_or(1.0, |state| state.window.scale_factor());
                // The renderer works in CSS pixels.
                self.pointer = ((position.x / scale) as f32, (position.y / scale) as f32);
                self.send_pointer(PointerKind::Move, PointerButton::None);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let scale = self
                    .window
                    .as_ref()
                    .map_or(1.0, |state| state.window.scale_factor());
                let (dx, dy) = wheel_delta(delta, scale, self.modifiers.shift);
                self.send(ToRenderer::Wheel {
                    dx,
                    dy,
                    x: self.pointer.0,
                    y: self.pointer.1,
                });
            }
            WindowEvent::CursorLeft { .. } => {
                self.send_pointer(PointerKind::Leave, PointerButton::None);
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let button = match button {
                    MouseButton::Left => PointerButton::Primary,
                    MouseButton::Right => PointerButton::Secondary,
                    MouseButton::Middle => PointerButton::Middle,
                    _ => return,
                };
                let kind = match state {
                    ElementState::Pressed => PointerKind::Down,
                    ElementState::Released => PointerKind::Up,
                };
                self.send_pointer(kind, button);
            }
            // Synthetic presses are keys already held when the window
            // gained the focus: not typed on the page.
            WindowEvent::KeyboardInput {
                event,
                is_synthetic: false,
                ..
            } => {
                self.send(ToRenderer::Key(KeyInput {
                    key: key(&event.logical_key),
                    state: match event.state {
                        ElementState::Pressed => KeyState::Down,
                        ElementState::Released => KeyState::Up,
                    },
                    modifiers: self.modifiers,
                }));
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                let state = modifiers.state();
                self.modifiers = Modifiers {
                    shift: state.shift_key(),
                    control: state.control_key(),
                    alt: state.alt_key(),
                    meta: state.super_key(),
                };
            }
            _ => {}
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Frame(frame) => {
                self.frame = Some(frame);
                if let Some(state) = &self.window {
                    state.window.request_redraw();
                }
            }
            UserEvent::Cursor(cursor) => {
                if let Some(state) = &self.window {
                    match icon(cursor) {
                        Some(icon) => {
                            state.window.set_cursor(icon);
                            state.window.set_cursor_visible(true);
                        }
                        None => state.window.set_cursor_visible(false),
                    }
                }
            }
            UserEvent::Raster(raster) => match raster {
                Raster::Gpu { adapter } => {
                    eprintln!("erk: drawing on the GPU: {adapter}");
                    // The renderer draws into the window from now on: a
                    // CPU frame kept from before would be blitted over it.
                    self.frame = None;
                    if let Some(state) = &mut self.window {
                        state.surface = None;
                    }
                }
                Raster::Cpu { reason } => eprintln!("erk: drawing on the CPU: {reason}"),
            },
            UserEvent::RendererGone => event_loop.exit(),
        }
    }
}

/// The window system's pointer for a CSS cursor; `None` hides it.
fn icon(cursor: Cursor) -> Option<CursorIcon> {
    Some(match cursor {
        Cursor::None => return None,
        Cursor::Default => CursorIcon::Default,
        Cursor::ContextMenu => CursorIcon::ContextMenu,
        Cursor::Help => CursorIcon::Help,
        Cursor::Pointer => CursorIcon::Pointer,
        Cursor::Progress => CursorIcon::Progress,
        Cursor::Wait => CursorIcon::Wait,
        Cursor::CellSelect => CursorIcon::Cell,
        Cursor::Crosshair => CursorIcon::Crosshair,
        Cursor::Text => CursorIcon::Text,
        Cursor::VerticalText => CursorIcon::VerticalText,
        Cursor::Alias => CursorIcon::Alias,
        Cursor::Copy => CursorIcon::Copy,
        Cursor::Move => CursorIcon::Move,
        Cursor::NoDrop => CursorIcon::NoDrop,
        Cursor::NotAllowed => CursorIcon::NotAllowed,
        Cursor::Grab => CursorIcon::Grab,
        Cursor::Grabbing => CursorIcon::Grabbing,
        Cursor::EResize => CursorIcon::EResize,
        Cursor::NResize => CursorIcon::NResize,
        Cursor::NeResize => CursorIcon::NeResize,
        Cursor::NwResize => CursorIcon::NwResize,
        Cursor::SResize => CursorIcon::SResize,
        Cursor::SeResize => CursorIcon::SeResize,
        Cursor::SwResize => CursorIcon::SwResize,
        Cursor::WResize => CursorIcon::WResize,
        Cursor::EwResize => CursorIcon::EwResize,
        Cursor::NsResize => CursorIcon::NsResize,
        Cursor::NeswResize => CursorIcon::NeswResize,
        Cursor::NwseResize => CursorIcon::NwseResize,
        Cursor::ColResize => CursorIcon::ColResize,
        Cursor::RowResize => CursorIcon::RowResize,
        Cursor::AllScroll => CursorIcon::AllScroll,
        Cursor::ZoomIn => CursorIcon::ZoomIn,
        Cursor::ZoomOut => CursorIcon::ZoomOut,
    })
}

/// How far a wheel turn scrolls per line it reports, in CSS pixels.
const LINE: f32 = 40.0;

/// A wheel turn as CSS pixels to scroll towards the end of the page: winit
/// reports lines (a wheel) or physical pixels (a touchpad), positive when
/// the content should move down, the other way round.
///
/// With Shift held a vertical turn scrolls sideways, as in browsers: a plain
/// wheel has no other way to reach a horizontal scroller. macOS turns it
/// itself and reports it sideways already, so only a purely vertical turn is
/// turned.
fn wheel_delta(delta: MouseScrollDelta, scale: f64, shift: bool) -> (f32, f32) {
    let (dx, dy) = match delta {
        MouseScrollDelta::LineDelta(x, y) => (-x * LINE, -y * LINE),
        MouseScrollDelta::PixelDelta(position) => {
            ((-position.x / scale) as f32, (-position.y / scale) as f32)
        }
    };
    if shift && dx == 0.0 {
        (dy, 0.0)
    } else {
        (dx, dy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = r#"<html style="background: #123456"></html>"#;

    #[test]
    fn a_wheel_turn_scrolls_towards_the_end_in_css_pixels() {
        use winit::dpi::PhysicalPosition;
        // A notch towards the user scrolls down.
        assert_eq!(
            wheel_delta(MouseScrollDelta::LineDelta(0.0, -1.0), 2.0, false),
            (0.0, LINE)
        );
        assert_eq!(
            wheel_delta(MouseScrollDelta::LineDelta(1.0, 0.0), 1.0, false),
            (-LINE, 0.0)
        );
        // A touchpad's physical pixels, at scale 2.
        assert_eq!(
            wheel_delta(
                MouseScrollDelta::PixelDelta(PhysicalPosition::new(10.0, -30.0)),
                2.0,
                false
            ),
            (-5.0, 15.0)
        );
    }

    #[test]
    fn shift_turns_a_vertical_wheel_sideways() {
        let notch = MouseScrollDelta::LineDelta(0.0, -1.0);
        assert_eq!(wheel_delta(notch, 1.0, true), (LINE, 0.0));
        assert_eq!(wheel_delta(notch, 1.0, false), (0.0, LINE));
        // A turn that is already sideways (a tilt wheel, a touchpad, or
        // macOS, which turns it itself) keeps its direction.
        let sideways = MouseScrollDelta::LineDelta(-1.0, 0.0);
        assert_eq!(wheel_delta(sideways, 1.0, true), (LINE, 0.0));
    }

    #[test]
    fn every_cursor_but_none_has_a_pointer() {
        assert_eq!(icon(Cursor::None), None);
        assert_eq!(icon(Cursor::Default), Some(CursorIcon::Default));
        assert_eq!(icon(Cursor::Pointer), Some(CursorIcon::Pointer));
        assert_eq!(icon(Cursor::Text), Some(CursorIcon::Text));
    }

    #[test]
    fn keys_reach_the_renderer_by_the_names_it_acts_on() {
        for (logical, expected) in [
            (WinitKey::Named(NamedKey::Tab), Key::Tab),
            (WinitKey::Named(NamedKey::Enter), Key::Enter),
            (WinitKey::Named(NamedKey::Space), Key::Space),
            (WinitKey::Character(" ".into()), Key::Space),
            (WinitKey::Named(NamedKey::Escape), Key::Escape),
            (
                WinitKey::Character("ş".into()),
                Key::Character("ş".to_owned()),
            ),
            (WinitKey::Named(NamedKey::ArrowDown), Key::Other),
        ] {
            assert_eq!(key(&logical), expected, "{logical:?}");
        }
    }

    /// A frame from the renderer thread, the only way the shell gets one.
    fn frame(width: u16, height: u16) -> Frame {
        let (to, from, renderer) = erk_renderer::spawn();
        to.send(ToRenderer::Load {
            html: PAGE.to_owned(),
        })
        .unwrap();
        to.send(ToRenderer::Resize { width, height }).unwrap();
        let Ok(FromRenderer::Frame(frame)) = from.recv() else {
            panic!("expected a frame");
        };
        to.send(ToRenderer::Shutdown).unwrap();
        renderer.join().unwrap();
        frame
    }

    #[test]
    fn a_renderer_panic_is_an_error_after_a_clean_event_loop_exit() {
        let panicked: std::thread::Result<()> = Err(Box::new("renderer panic"));
        assert_eq!(
            finish(Ok(()), panicked),
            Err("the renderer thread panicked".to_owned())
        );
        assert_eq!(finish(Ok(()), Ok(())), Ok(()));
    }

    #[test]
    fn blit_converts_to_xrgb_and_clips_to_the_buffer() {
        let frame = frame(4, 4);
        let mut buffer = vec![0u32; 3 * 2];
        blit(&frame, &mut buffer, 3, 2);
        assert!(
            buffer.iter().all(|&pixel| pixel == 0x0012_3456),
            "{buffer:x?}"
        );
    }

    #[test]
    fn blit_leaves_the_rest_of_a_larger_buffer_alone() {
        let frame = frame(2, 1);
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
