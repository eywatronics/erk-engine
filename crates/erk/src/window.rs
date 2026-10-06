//! An app's window and its event loop (p1-contract §7: Erk's own loop):
//! winit for the window and its input, the raster drawing into it on the
//! GPU, or softbuffer putting the raster's CPU frames on screen.
//!
//! The loop runs on the UI thread, the app's. Input goes to the app as it
//! comes; once the loop has nothing left to do it ticks the app, which
//! prepares a frame if something that shows has changed and hands it to the
//! raster. What the raster paints, and messages other threads send the app,
//! come back as user events that wake the loop.

use std::num::NonZeroU32;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::Arc;
use std::time::Instant;

use erk_renderer::{Cursor, Frame, Painted, Raster, RasterThread};
use softbuffer::{Context as SoftContext, Surface};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::keyboard::{Key as WinitKey, NamedKey};
use winit::window::{CursorIcon, Window, WindowId};

use crate::{
    App, Input, Key, KeyInput, KeyState, LogLevel, Modifiers, Output, PointerButton, PointerInput,
    PointerKind, RunError,
};

enum UserEvent {
    /// What the raster painted.
    Painted(Painted),
    /// Another thread sent the app a message: tick it.
    Wake,
}

/// Run `app`'s window until it closes.
pub(crate) fn run(app: &mut App) -> Result<(), RunError> {
    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .map_err(|error| RunError(error.to_string()))?;
    let proxy = event_loop.create_proxy();
    let waker = proxy.clone();
    // Fails harmlessly once the loop has ended.
    app.cx.wake_with(move || {
        let _ = waker.send_event(UserEvent::Wake);
    });
    let mut host = Host {
        app,
        proxy,
        window: None,
        frame: None,
        pointer: (0.0, 0.0),
        modifiers: Modifiers::default(),
        started: Instant::now(),
        cursor: None,
        panic: None,
    };
    let result = event_loop.run_app(&mut host);
    let panic = host.panic.take();
    // The raster stops with the window.
    host.app.output = Output::Window(None);
    if let Some(panic) = panic {
        resume_unwind(panic);
    }
    result.map_err(|error| RunError(error.to_string()))
}

struct WindowState {
    window: Arc<Window>,
    /// softbuffer's surface, made when the first CPU frame arrives: on the
    /// GPU path the raster draws into the window itself.
    surface: Option<Surface<Arc<Window>, Arc<Window>>>,
}

struct Host<'a> {
    app: &'a mut App,
    proxy: EventLoopProxy<UserEvent>,
    window: Option<WindowState>,
    /// The latest CPU frame, kept for redraws.
    frame: Option<Frame>,
    /// Where the pointer is, in CSS pixels, and the modifier keys held: a
    /// button event carries neither.
    pointer: (f32, f32),
    modifiers: Modifiers,
    /// The loop's clock, the one the app's ticks get.
    started: Instant,
    /// The cursor the window shows.
    cursor: Option<Cursor>,
    /// A panic in the host's callbacks, carried out of the loop.
    panic: Option<Box<dyn std::any::Any + Send>>,
}

impl Host<'_> {
    /// Run `work` on the app; a panic in it ends the loop and comes out of
    /// `run` (p1-contract §8), instead of unwinding through the platform's
    /// event loop.
    fn guarded(&mut self, event_loop: &ActiveEventLoop, work: impl FnOnce(&mut App)) {
        if self.panic.is_some() {
            return;
        }
        if let Err(panic) = catch_unwind(AssertUnwindSafe(|| work(self.app))) {
            self.panic = Some(panic);
            event_loop.exit();
        }
    }

    fn input(&mut self, event_loop: &ActiveEventLoop, input: Input) {
        self.guarded(event_loop, |app| app.input(input));
    }

    fn pointer(&mut self, event_loop: &ActiveEventLoop, kind: PointerKind, button: PointerButton) {
        let input = Input::Pointer(PointerInput {
            kind,
            x: self.pointer.0,
            y: self.pointer.1,
            button,
            modifiers: self.modifiers,
        });
        self.input(event_loop, input);
    }

    /// The engine lays out at the window's scale (device pixels per CSS
    /// pixel), in its size in device pixels: a page laid out for 800 CSS
    /// pixels fills an 800-point window on any screen, sharply.
    fn fit(&mut self, window: &Window) {
        let clamp = |v: u32| u16::try_from(v).unwrap_or(u16::MAX);
        let size = window.inner_size();
        let engine = &mut self.app.cx.engine;
        engine.set_scale(window.scale_factor() as f32);
        engine.resize(clamp(size.width), clamp(size.height));
    }

    fn scale(&self) -> f64 {
        self.window
            .as_ref()
            .map_or(1.0, |state| state.window.scale_factor())
    }

    fn redraw(&mut self) {
        // Only frames the CPU painted are this window's to show.
        let (Some(state), Some(frame)) = (&mut self.window, &self.frame) else {
            return;
        };
        if state.surface.is_none() {
            let surface = SoftContext::new(state.window.clone())
                .and_then(|context| Surface::new(&context, state.window.clone()));
            match surface {
                Ok(surface) => state.surface = Some(surface),
                Err(error) => {
                    let message = format!("cannot draw into the window: {error}");
                    self.app.log(LogLevel::Error, &message);
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
        // White until the raster has caught up with the window's size.
        buffer.fill(0x00ff_ffff);
        blit(frame, &mut buffer, width, height);
        let _ = buffer.present();
    }

    /// Show the cursor the page asks for, when it changes.
    fn show_cursor(&mut self) {
        let now = self.app.cx.engine.cursor();
        if self.cursor == Some(now) {
            return;
        }
        self.cursor = Some(now);
        if let Some(state) = &self.window {
            match icon(now) {
                Some(icon) => {
                    state.window.set_cursor(icon);
                    state.window.set_cursor_visible(true);
                }
                None => state.window.set_cursor_visible(false),
            }
        }
    }
}

impl ApplicationHandler<UserEvent> for Host<'_> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let config = &self.app.config;
        let attributes = Window::default_attributes()
            .with_title(&config.title)
            .with_inner_size(LogicalSize::new(config.width, config.height));
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                let message = format!("cannot create a window: {error}");
                self.app.log(LogLevel::Error, &message);
                event_loop.exit();
                return;
            }
        };
        let proxy = self.proxy.clone();
        let sink = move |painted| {
            let _ = proxy.send_event(UserEvent::Painted(painted));
        };
        // Made here, on the window's thread: some platforms give a window's
        // handle only there.
        let raster = if self.app.config.gpu {
            RasterThread::on_window(window.clone(), sink)
        } else {
            RasterThread::cpu(sink)
        };
        self.app.output = Output::Window(Some(raster));
        self.fit(&window);
        self.window = Some(WindowState {
            window,
            surface: None,
        });
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                if let Some(window) = self.window.as_ref().map(|state| state.window.clone()) {
                    self.fit(&window);
                    window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => self.redraw(),
            WindowEvent::CursorMoved { position, .. } => {
                // The engine works in CSS pixels.
                let scale = self.scale();
                self.pointer = ((position.x / scale) as f32, (position.y / scale) as f32);
                self.pointer(event_loop, PointerKind::Move, PointerButton::None);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let (dx, dy) = wheel_delta(delta, self.scale(), self.modifiers.shift);
                let (x, y) = self.pointer;
                self.input(event_loop, Input::Wheel { dx, dy, x, y });
            }
            WindowEvent::CursorLeft { .. } => {
                self.pointer(event_loop, PointerKind::Leave, PointerButton::None);
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
                self.pointer(event_loop, kind, button);
            }
            // Synthetic presses are keys already held when the window
            // gained the focus: not typed on the page.
            WindowEvent::KeyboardInput {
                event,
                is_synthetic: false,
                ..
            } => {
                let input = Input::Key(KeyInput {
                    key: key(&event.logical_key),
                    state: match event.state {
                        ElementState::Pressed => KeyState::Down,
                        ElementState::Released => KeyState::Up,
                    },
                    modifiers: self.modifiers,
                });
                self.input(event_loop, input);
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

    fn user_event(&mut self, _: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Painted(Painted::Frame(frame)) => {
                self.app.painted();
                self.frame = Some(frame);
                if let Some(state) = &self.window {
                    state.window.request_redraw();
                }
            }
            UserEvent::Painted(Painted::Presented { .. }) => self.app.painted(),
            UserEvent::Painted(Painted::Raster(Raster::Gpu { adapter })) => {
                self.app
                    .log(LogLevel::Info, &format!("drawing on the GPU: {adapter}"));
                // The raster draws into the window from now on: a CPU frame
                // kept from before would be blitted over it.
                self.frame = None;
                if let Some(state) = &mut self.window {
                    state.surface = None;
                }
            }
            UserEvent::Painted(Painted::Raster(Raster::Cpu { reason })) => {
                self.app
                    .log(LogLevel::Info, &format!("drawing on the CPU: {reason}"));
            }
            // The tick after this event handles what arrived.
            UserEvent::Wake => {}
        }
    }

    /// Everything that came in is handled: a turn of the app.
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = u64::try_from(self.started.elapsed().as_nanos()).unwrap_or(u64::MAX);
        self.guarded(event_loop, |app| app.tick(now));
        self.show_cursor();
    }
}

/// The engine's name for a key winit reports.
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
    use crate::Config;

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
    fn keys_reach_the_engine_by_the_names_it_acts_on() {
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

    /// A frame of `PAGE`, painted as a windowless app paints it.
    fn frame(width: u32, height: u32) -> Frame {
        let mut app = App::headless(Config {
            width,
            height,
            system_fonts: false,
            ..Config::default()
        })
        .unwrap();
        app.load_html(PAGE);
        app.tick(0);
        let Output::Headless { frame, .. } =
            std::mem::replace(&mut app.output, Output::Window(None))
        else {
            unreachable!("a windowless app");
        };
        frame.expect("a frame")
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

    #[test]
    fn only_a_windowed_app_runs() {
        let mut app = App::headless(Config {
            system_fonts: false,
            ..Config::default()
        })
        .unwrap();
        assert!(app.run().is_err());
    }
}
