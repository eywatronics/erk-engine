//! Erk's C ABI (p1-contract): the `erk` crate's API as plain C functions,
//! declared in `include/erk.h`, which cbindgen generates from this file.
//!
//! Every exported function goes through [`guard`] (CI checks it): a null
//! app is an invalid argument; a call from a thread other than the app's UI
//! thread does nothing and returns `ERK_ERR_WRONG_THREAD` (§4); a call that
//! may not run inside a callback returns `ERK_ERR_REENTRANT` there (§5); a
//! panic never crosses the boundary, it returns `ERK_ERR_PANIC` and poisons
//! the app, after which every call but `erk_app_destroy` returns
//! `ERK_ERR_POISONED` (§8). Two functions may be called from any thread:
//! `erk_app_post` and `erk_resource_complete`.
//!
//! Strings come in as UTF-8 `ErkStr`s, copied before the call returns.
//! Small results go to the caller's buffer, whole or not at all
//! (`ERK_ERR_BUFFER_TOO_SMALL`, with the size needed); large ones come as an
//! `ErkString` the caller gives back to `erk_string_free` (§3). A structure
//! the host fills or Erk fills starts with its size: a shorter one, from an
//! older header, has its missing fields defaulted or left unwritten (§2).
//!
//! The unsafe code here is what a C ABI is: reading the pointers C passes
//! and exporting unmangled symbols. Each item that needs it allows it, and
//! says why.
//!
//! The exported functions take C's raw pointers and are not `unsafe fn`:
//! C, their caller, has no `unsafe`, and their pointer contract is the
//! header's (§2, §3). Rust hosts use the `erk` crate; this crate's Rust
//! library is for its own tests.
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::cell::{Cell, UnsafeCell};
use std::collections::HashMap;
use std::ffi::{c_char, c_void};
use std::mem::{offset_of, size_of};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::ThreadId;

use erk::{
    App, AppHandle, Config, Context, Event, EventKind, Input, Key, KeyInput, KeyState, LogLevel,
    Modifiers, Mutation, Node, NodeKind, Phase, PointerButton, PointerInput, PointerKind, Ref,
    ResourceKind, Responder, Status, Subscription,
};

// ---- Versions, status codes --------------------------------------------------

/// The ABI's version: `(major << 16) | minor`. Within a major version
/// functions, constants and trailing structure fields are only added.
pub const ERK_ABI_VERSION: u32 = 6;

/// What a call did: `ERK_OK`, or why it failed.
pub type ErkStatus = i32;
pub const ERK_OK: ErkStatus = 0;
/// A null pointer, bad UTF-8, a value out of range.
pub const ERK_ERR_INVALID_ARGUMENT: ErkStatus = 1;
/// The node was removed, belongs to a replaced document or to another app.
pub const ERK_ERR_STALE_NODE: ErkStatus = 2;
/// Called from a thread other than the app's UI thread; nothing was done.
pub const ERK_ERR_WRONG_THREAD: ErkStatus = 3;
/// The caller's buffer is too small; `*len` holds the size needed.
pub const ERK_ERR_BUFFER_TOO_SMALL: ErkStatus = 4;
pub const ERK_ERR_NOT_FOUND: ErkStatus = 5;
/// Not allowed inside a callback.
pub const ERK_ERR_REENTRANT: ErkStatus = 6;
/// The call panicked; the app is now poisoned.
pub const ERK_ERR_PANIC: ErkStatus = 7;
/// An earlier call panicked; only `erk_app_destroy` works.
pub const ERK_ERR_POISONED: ErkStatus = 8;
/// The platform could not make the window or run its event loop.
pub const ERK_ERR_PLATFORM: ErkStatus = 9;

// ---- Basic types -------------------------------------------------------------

/// A node of an app's document: opaque, meaningful only to the app that
/// gave it, never 0. Not a security token.
pub type ErkNodeId = u64;
/// No node.
pub const ERK_NODE_NONE: ErkNodeId = 0;

/// UTF-8 text in: Erk copies it before the call returns. It need not end
/// with NUL; `ptr` may be null when `len` is 0.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ErkStr {
    pub ptr: *const c_char,
    pub len: usize,
}

/// Text Erk owns: give it back to `erk_string_free`.
#[repr(C)]
pub struct ErkString {
    pub ptr: *mut c_char,
    pub len: usize,
}

/// An app: one document and its window. Opaque.
pub struct ErkApp {
    /// The UI thread: the one that made the app.
    thread: ThreadId,
    poisoned: AtomicBool,
    /// The context of the callback running now, if one is: calls from
    /// inside it go there (§5).
    current: Cell<Option<NonNull<Context>>>,
    /// The way in from other threads (§4).
    handle: AppHandle,
    /// Resource requests the host has not answered yet, for any thread.
    pending: Arc<Mutex<HashMap<u64, Responder>>>,
    /// The app; only ever touched on the UI thread. Set once made, before
    /// the host sees the pointer.
    app: UnsafeCell<Option<App>>,
}

/// A subscription, to cancel with `erk_off`; never 0.
pub type ErkSubscription = u64;

// ---- Callbacks ---------------------------------------------------------------

pub type ErkDestroyFn = Option<unsafe extern "C" fn(user_data: *mut c_void)>;
pub type ErkPostFn = Option<unsafe extern "C" fn(user_data: *mut c_void, app: *mut ErkApp)>;
/// Content asks for a resource (CSS `url()`, `<img>`). Answer now or later,
/// from any thread, with `erk_resource_complete`.
pub type ErkResourceFn = Option<
    unsafe extern "C" fn(
        user_data: *mut c_void,
        app: *mut ErkApp,
        request: u64,
        kind: u32,
        url: ErkStr,
    ),
>;
pub type ErkLogFn =
    Option<unsafe extern "C" fn(user_data: *mut c_void, level: u32, message: ErkStr)>;
pub type ErkEventFn =
    Option<unsafe extern "C" fn(user_data: *mut c_void, app: *mut ErkApp, event: *const ErkEvent)>;

pub const ERK_RESOURCE_IMAGE: u32 = 1;
pub const ERK_RESOURCE_STYLESHEET: u32 = 2;
pub const ERK_RESOURCE_FONT: u32 = 3;

pub const ERK_LOG_ERROR: u32 = 1;
pub const ERK_LOG_WARNING: u32 = 2;
pub const ERK_LOG_INFO: u32 = 3;
pub const ERK_LOG_DEBUG: u32 = 4;

// ---- Application -------------------------------------------------------------

/// No window: the host ticks the app, gives it input and reads its frames.
pub const ERK_APP_HEADLESS: u32 = 1;
/// Draw text with the embedded font only, the same on every machine.
pub const ERK_APP_EMBEDDED_FONTS: u32 = 2;
/// Draw the window on the CPU even where a GPU is available.
pub const ERK_APP_CPU: u32 = 4;

/// What an app starts with. Fields past `struct_size` take their defaults.
#[repr(C)]
pub struct ErkConfig {
    /// `sizeof(ErkConfig)`.
    pub struct_size: u32,
    /// The viewport in logical pixels.
    pub width: u32,
    pub height: u32,
    /// Device pixels per CSS pixel of a headless app; 0 is 1.
    pub scale: f32,
    pub title: ErkStr,
    /// May be NULL: no resource loads.
    pub resource: ErkResourceFn,
    pub resource_user_data: *mut c_void,
    /// May be NULL.
    pub log: ErkLogFn,
    pub log_user_data: *mut c_void,
    /// The least severe `ERK_LOG_*` the log gets; 0 is `ERK_LOG_WARNING`.
    pub log_level: u32,
    /// `ERK_APP_*` flags.
    pub flags: u32,
}

/// A frame of a headless app: `width` × `height` premultiplied RGBA8
/// pixels, valid until the next tick or the app's end.
#[repr(C)]
pub struct ErkFrame {
    pub struct_size: u32,
    pub width: u32,
    pub height: u32,
    pub rgba: *const u8,
    pub len: usize,
}

pub const ERK_INPUT_POINTER_MOVE: u32 = 1;
pub const ERK_INPUT_POINTER_DOWN: u32 = 2;
pub const ERK_INPUT_POINTER_UP: u32 = 3;
pub const ERK_INPUT_POINTER_LEAVE: u32 = 4;
pub const ERK_INPUT_KEY_DOWN: u32 = 5;
pub const ERK_INPUT_KEY_UP: u32 = 6;
pub const ERK_INPUT_WHEEL: u32 = 7;

pub const ERK_BUTTON_NONE: u32 = 0;
pub const ERK_BUTTON_PRIMARY: u32 = 1;
pub const ERK_BUTTON_SECONDARY: u32 = 2;
pub const ERK_BUTTON_MIDDLE: u32 = 3;

pub const ERK_KEY_OTHER: u32 = 0;
pub const ERK_KEY_TAB: u32 = 1;
pub const ERK_KEY_ENTER: u32 = 2;
pub const ERK_KEY_SPACE: u32 = 3;
pub const ERK_KEY_ESCAPE: u32 = 4;
/// The key typed `text`.
pub const ERK_KEY_CHARACTER: u32 = 5;
pub const ERK_KEY_BACKSPACE: u32 = 6;

pub const ERK_MOD_SHIFT: u32 = 1;
pub const ERK_MOD_CONTROL: u32 = 2;
pub const ERK_MOD_ALT: u32 = 4;
pub const ERK_MOD_META: u32 = 8;

/// Input for a headless app, in CSS pixels. Fields past `struct_size` are 0.
#[repr(C)]
pub struct ErkInput {
    pub struct_size: u32,
    /// `ERK_INPUT_*`.
    pub kind: u32,
    pub x: f32,
    pub y: f32,
    /// How far a wheel scrolls; positive towards the end of the page.
    pub dx: f32,
    pub dy: f32,
    /// `ERK_BUTTON_*`.
    pub button: u32,
    /// `ERK_KEY_*`.
    pub key: u32,
    /// The character an `ERK_KEY_CHARACTER` typed.
    pub text: ErkStr,
    /// `ERK_MOD_*` bits.
    pub modifiers: u32,
}

// ---- Events ------------------------------------------------------------------

pub const ERK_EVENT_CLICK: u32 = 1;
pub const ERK_EVENT_INPUT: u32 = 2;
pub const ERK_EVENT_CHANGE: u32 = 3;
pub const ERK_EVENT_SUBMIT: u32 = 4;
pub const ERK_EVENT_KEY_DOWN: u32 = 5;
pub const ERK_EVENT_KEY_UP: u32 = 6;
pub const ERK_EVENT_FOCUS: u32 = 7;
pub const ERK_EVENT_BLUR: u32 = 8;

pub const ERK_PHASE_CAPTURE: u32 = 1;
pub const ERK_PHASE_TARGET: u32 = 2;
pub const ERK_PHASE_BUBBLE: u32 = 3;

/// An event, valid only during the callback.
#[repr(C)]
pub struct ErkEvent {
    pub struct_size: u32,
    /// `ERK_EVENT_*`; ignore kinds you do not know.
    pub kind: u32,
    /// `ERK_PHASE_*`.
    pub phase: u32,
    pub target: ErkNodeId,
    pub current_target: ErkNodeId,
    /// Logical pixels, pointer events.
    pub x: f64,
    pub y: f64,
    /// `ERK_MOD_*` bits.
    pub modifiers: u32,
    /// Key events: the character typed, for `ERK_KEY_CHARACTER`.
    pub text: ErkStr,
    /// Key events: `ERK_KEY_*`.
    pub key: u32,
}

// ---- Mutations ---------------------------------------------------------------

pub const ERK_MUTATION_CREATE_ELEMENT: u32 = 1;
pub const ERK_MUTATION_CREATE_TEXT: u32 = 2;
pub const ERK_MUTATION_APPEND: u32 = 3;
pub const ERK_MUTATION_INSERT_BEFORE: u32 = 4;
pub const ERK_MUTATION_REMOVE: u32 = 5;
pub const ERK_MUTATION_SET_TEXT: u32 = 6;
pub const ERK_MUTATION_SET_ATTR: u32 = 7;
pub const ERK_MUTATION_REMOVE_ATTR: u32 = 8;
pub const ERK_MUTATION_ADD_CLASS: u32 = 9;
pub const ERK_MUTATION_REMOVE_CLASS: u32 = 10;

/// A node an earlier mutation of the same batch created, by its position:
/// `ERK_NEW_NODE + i` names what mutation `i` created. No real node id is
/// this small: a node id's upper half is its generation, never 0.
pub const ERK_NEW_NODE: ErkNodeId = 1;

/// One change of a batch for `erk_apply`. `node`, `parent` and `before` are
/// node ids or `ERK_NEW_NODE + i`.
///
/// - `ERK_MUTATION_CREATE_ELEMENT`: `value` is the tag.
/// - `ERK_MUTATION_CREATE_TEXT`: `value` is the text.
/// - `ERK_MUTATION_APPEND`: `node` into `parent`, last.
/// - `ERK_MUTATION_INSERT_BEFORE`: `node` into `parent` before `before`
///   (`ERK_NODE_NONE`: last).
/// - `ERK_MUTATION_REMOVE`: `node` and everything in it.
/// - `ERK_MUTATION_SET_TEXT`: `node`'s text to `value`.
/// - `ERK_MUTATION_SET_ATTR`: attribute `name` of `node` to `value`.
/// - `ERK_MUTATION_REMOVE_ATTR`: attribute `name` of `node`.
/// - `ERK_MUTATION_ADD_CLASS`, `ERK_MUTATION_REMOVE_CLASS`: class `value`.
#[repr(C)]
pub struct ErkMutation {
    /// `sizeof(ErkMutation)`: also the stride of the array.
    pub struct_size: u32,
    /// `ERK_MUTATION_*`.
    pub kind: u32,
    pub node: ErkNodeId,
    pub parent: ErkNodeId,
    pub before: ErkNodeId,
    pub name: ErkStr,
    pub value: ErkStr,
}

// ---- Inspection --------------------------------------------------------------

pub const ERK_NODE_DOCUMENT: u32 = 1;
pub const ERK_NODE_ELEMENT: u32 = 2;
pub const ERK_NODE_TEXT: u32 = 3;
pub const ERK_NODE_COMMENT: u32 = 4;
pub const ERK_NODE_OTHER: u32 = 5;

/// A node's box in the last frame: the border box in CSS pixels relative to
/// the viewport, and the sides, each top, right, bottom, left.
#[repr(C)]
pub struct ErkBox {
    pub struct_size: u32,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub margin: [f32; 4],
    pub border: [f32; 4],
    pub padding: [f32; 4],
}

/// How long the last frame's stages took, measured outside the core.
#[repr(C)]
pub struct ErkFrameTimings {
    pub struct_size: u32,
    pub frame: u64,
    pub style_ns: u64,
    pub layout_ns: u64,
    pub display_list_ns: u64,
    pub raster_ns: u64,
}

// ---- The guard ---------------------------------------------------------------

/// What a call may do.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Calls {
    /// Anything but the loop: also from inside a callback.
    Document,
    /// The loop and the app's end: not from inside a callback.
    Loop,
}

/// Run `work` on `app` as p1-contract §4, §5 and §8 say: on its UI thread
/// only, not poisoned, not re-entered where that is not allowed, and with
/// any panic turned into `ERK_ERR_PANIC`, which poisons the app.
#[allow(unsafe_code)] // SAFETY: reads the thread and flags through the caller's app pointer, checked non-null.
fn guard(
    app: *mut ErkApp,
    calls: Calls,
    work: impl FnOnce(&ErkApp) -> Result<(), ErkStatus>,
) -> ErkStatus {
    if app.is_null() {
        return ERK_ERR_INVALID_ARGUMENT;
    }
    // SAFETY: a non-null app pointer is one erk_app_create returned and
    // erk_app_destroy has not taken back (the caller's promise, §2); only
    // fields any thread may read are read before the thread check.
    let shared = unsafe { &*app };
    if shared.thread != std::thread::current().id() {
        return ERK_ERR_WRONG_THREAD;
    }
    if shared.poisoned.load(Ordering::Acquire) {
        return ERK_ERR_POISONED;
    }
    if calls == Calls::Loop && shared.current.get().is_some() {
        return ERK_ERR_REENTRANT;
    }
    match catch_unwind(AssertUnwindSafe(|| work(shared))) {
        Ok(Ok(())) => ERK_OK,
        Ok(Err(status)) => status,
        Err(_) => {
            shared.poisoned.store(true, Ordering::Release);
            ERK_ERR_PANIC
        }
    }
}

/// Run `work` without an app: a panic is `ERK_ERR_PANIC`, nothing to poison.
fn guard_free(work: impl FnOnce() -> Result<(), ErkStatus>) -> ErkStatus {
    match catch_unwind(AssertUnwindSafe(work)) {
        Ok(Ok(())) => ERK_OK,
        Ok(Err(status)) => status,
        Err(_) => ERK_ERR_PANIC,
    }
}

impl ErkApp {
    /// The document API: the running callback's context, or the app's.
    // SAFETY: the app and the callback's context are the UI thread's, checked by the guard.
    // The app lives in an UnsafeCell: the guard confines it to the UI thread,
    // and each call takes one borrow for as long as it runs.
    #[allow(unsafe_code, clippy::mut_from_ref)]
    fn cx(&self) -> &mut Context {
        match self.current.get() {
            // SAFETY: set by the callback that is running, for the time it
            // runs, from the `&mut Context` it was given.
            Some(mut cx) => unsafe { cx.as_mut() },
            // SAFETY: no callback runs, so nothing else borrows the app;
            // the guard checked the thread.
            None => unsafe { &mut *self.app.get() }
                .as_mut()
                .expect("an app is made before its pointer is given out"),
        }
    }

    /// The app itself, for the loop: never from inside a callback (the
    /// guard checks it).
    // SAFETY: the UI thread's, outside any callback, checked by the guard.
    // The app lives in an UnsafeCell: the guard confines it to the UI thread,
    // and each call takes one borrow for as long as it runs.
    #[allow(unsafe_code, clippy::mut_from_ref)]
    fn app(&self) -> &mut App {
        // SAFETY: as `cx`, and no callback is running.
        unsafe { &mut *self.app.get() }
            .as_mut()
            .expect("an app is made before its pointer is given out")
    }

    /// Run `callback` with `cx` as the context calls go to.
    fn calling(&self, cx: &mut Context, callback: impl FnOnce()) {
        let before = self.current.replace(Some(NonNull::from(cx)));
        callback();
        self.current.set(before);
    }
}

// ---- Raw values in and out ---------------------------------------------------

/// The text `s` holds, copied.
#[allow(unsafe_code)] // SAFETY: reads the bytes C passes as (ptr, len).
fn text(s: ErkStr) -> Result<String, ErkStatus> {
    if s.len == 0 {
        return Ok(String::new());
    }
    if s.ptr.is_null() {
        return Err(ERK_ERR_INVALID_ARGUMENT);
    }
    // SAFETY: the caller promises `len` readable bytes at `ptr` (§3).
    let bytes = unsafe { std::slice::from_raw_parts(s.ptr.cast::<u8>(), s.len) };
    String::from_utf8(bytes.to_vec()).map_err(|_| ERK_ERR_INVALID_ARGUMENT)
}

/// `s` lent to C for a callback.
fn lent(s: &str) -> ErkStr {
    ErkStr {
        ptr: s.as_ptr().cast(),
        len: s.len(),
    }
}

/// Write `value` to `out`.
#[allow(unsafe_code)] // SAFETY: writes through the out pointer C passes, checked non-null.
fn put<T>(out: *mut T, value: T) -> Result<(), ErkStatus> {
    if out.is_null() {
        return Err(ERK_ERR_INVALID_ARGUMENT);
    }
    // SAFETY: the caller gives a pointer to a writable T.
    unsafe { out.write(value) };
    Ok(())
}

/// `text` into the caller's buffer, whole or not at all; `*len` is always
/// the size needed.
#[allow(unsafe_code)] // SAFETY: writes up to `cap` bytes into the caller's buffer.
fn put_text(text: &str, buf: *mut c_char, cap: usize, len: *mut usize) -> Result<(), ErkStatus> {
    put(len, text.len())?;
    if cap < text.len() {
        return Err(ERK_ERR_BUFFER_TOO_SMALL);
    }
    if text.is_empty() {
        return Ok(());
    }
    if buf.is_null() {
        return Err(ERK_ERR_INVALID_ARGUMENT);
    }
    // SAFETY: the caller gives `cap` writable bytes at `buf`, and `cap` is
    // at least the text's length.
    unsafe { std::ptr::copy_nonoverlapping(text.as_ptr(), buf.cast::<u8>(), text.len()) };
    Ok(())
}

/// `text` as an `ErkString` the caller frees.
fn owned(text: String) -> ErkString {
    let bytes = text.into_bytes().into_boxed_slice();
    let len = bytes.len();
    ErkString {
        ptr: Box::into_raw(bytes).cast::<c_char>(),
        len,
    }
}

/// Write the part of `value` the caller's structure has room for: its
/// `struct_size`, which it keeps.
#[allow(unsafe_code)] // SAFETY: reads the caller's struct_size, then writes no more bytes than it says.
fn put_sized<T>(out: *mut T, value: T) -> Result<(), ErkStatus> {
    if out.is_null() {
        return Err(ERK_ERR_INVALID_ARGUMENT);
    }
    // SAFETY: every sized structure starts with its `uint32_t struct_size`,
    // which the caller set.
    let room = unsafe { out.cast::<u32>().read() } as usize;
    if room < size_of::<u32>() {
        return Err(ERK_ERR_INVALID_ARGUMENT);
    }
    let size = room.min(size_of::<T>());
    // SAFETY: the caller's structure has `room` writable bytes; the first
    // four (struct_size) are left as they are.
    unsafe {
        let from = (&raw const value).cast::<u8>();
        std::ptr::copy_nonoverlapping(
            from.add(size_of::<u32>()),
            out.cast::<u8>().add(size_of::<u32>()),
            size - size_of::<u32>(),
        );
    }
    Ok(())
}

/// Field `$field` of the caller's structure `$from`, if its `struct_size`
/// reaches it; else `$default`.
macro_rules! field {
    ($from:expr, $type:ty, $field:ident, $default:expr) => {{
        let end = offset_of!($type, $field) + size_of_field::<$type, _>(|s| &s.$field);
        #[allow(unsafe_code)] // SAFETY: reads a field the caller's struct_size covers.
        let value = if (unsafe { (*$from).struct_size } as usize) >= end {
            // SAFETY: the caller's structure is at least `end` bytes long.
            unsafe { (&raw const (*$from).$field).read_unaligned() }
        } else {
            $default
        };
        value
    }};
}

const fn size_of_field<T, F>(_: fn(&T) -> &F) -> usize {
    size_of::<F>()
}

fn node(id: ErkNodeId) -> Result<Node, ErkStatus> {
    Node::from_raw(id).ok_or(ERK_ERR_INVALID_ARGUMENT)
}

fn raw(node: Option<Node>) -> ErkNodeId {
    node.map_or(ERK_NODE_NONE, Node::to_raw)
}

fn status(status: Status) -> ErkStatus {
    status as ErkStatus
}

/// `Result<T, Status>` as the ABI's status.
trait AbiResult<T> {
    fn abi(self) -> Result<T, ErkStatus>;
}

impl<T> AbiResult<T> for Result<T, Status> {
    fn abi(self) -> Result<T, ErkStatus> {
        self.map_err(status)
    }
}

/// A raw pointer that may cross to the UI thread: the host's `user_data`,
/// and the app it was posted to, for a call that runs there.
struct Posted(*mut c_void, *mut ErkApp);

// SAFETY: the pointers are only used on the app's UI thread, where the host
// posted them to be used (§4: erk_app_post's contract).
#[allow(unsafe_code)]
unsafe impl Send for Posted {}

/// The host's `destroy`, called once when what holds it is dropped (§5).
struct Destroy(ErkDestroyFn, *mut c_void);

impl Drop for Destroy {
    #[allow(unsafe_code)] // SAFETY: calls the host's destroy with its own user_data, once.
    fn drop(&mut self) {
        if let Some(destroy) = self.0 {
            // SAFETY: the host gave this function and pointer together.
            unsafe { destroy(self.1) };
        }
    }
}

// SAFETY: a posted destroy runs on the UI thread with the posted work.
#[allow(unsafe_code)]
unsafe impl Send for Destroy {}

// ---- Exported functions ------------------------------------------------------

#[allow(unsafe_code)] // SAFETY: an exported symbol; takes and returns plain values.
#[unsafe(no_mangle)]
pub extern "C" fn erk_abi_version() -> u32 {
    ERK_ABI_VERSION
}

/// Give back a string Erk made. A null string is ignored.
#[allow(unsafe_code)] // SAFETY: takes back the allocation `owned` made.
#[unsafe(no_mangle)]
pub extern "C" fn erk_string_free(s: ErkString) {
    let _ = guard_free(|| {
        if !s.ptr.is_null() {
            // SAFETY: `owned` made (ptr, len) from a boxed byte slice.
            drop(unsafe {
                Box::from_raw(std::ptr::slice_from_raw_parts_mut(
                    s.ptr.cast::<u8>(),
                    s.len,
                ))
            });
        }
        Ok(())
    });
}

/// Make an app on this thread, which becomes its UI thread.
#[allow(unsafe_code)] // SAFETY: an exported symbol; reads the caller's config within its struct_size.
#[unsafe(no_mangle)]
pub extern "C" fn erk_app_create(config: *const ErkConfig, out: *mut *mut ErkApp) -> ErkStatus {
    guard_free(|| {
        if config.is_null() || out.is_null() {
            return Err(ERK_ERR_INVALID_ARGUMENT);
        }
        // SAFETY: the caller's config starts with its struct_size.
        let size = unsafe { (*config).struct_size } as usize;
        if size < offset_of!(ErkConfig, scale) {
            return Err(ERK_ERR_INVALID_ARGUMENT);
        }
        let none = ErkStr {
            ptr: std::ptr::null(),
            len: 0,
        };
        let width = field!(config, ErkConfig, width, 0);
        let height = field!(config, ErkConfig, height, 0);
        let scale = field!(config, ErkConfig, scale, 0.0);
        let title = text(field!(config, ErkConfig, title, none))?;
        let resource = field!(config, ErkConfig, resource, None);
        let resource_user_data =
            field!(config, ErkConfig, resource_user_data, std::ptr::null_mut());
        let log = field!(config, ErkConfig, log, None);
        let log_user_data = field!(config, ErkConfig, log_user_data, std::ptr::null_mut());
        let log_level = field!(config, ErkConfig, log_level, 0);
        let flags = field!(config, ErkConfig, flags, 0);
        let defaults = Config::default();
        let config = Config {
            width,
            height,
            scale: if scale == 0.0 { 1.0 } else { scale },
            title: if title.is_empty() {
                defaults.title.clone()
            } else {
                title
            },
            log_level: match log_level {
                0 => LogLevel::Warning,
                ERK_LOG_ERROR => LogLevel::Error,
                ERK_LOG_WARNING => LogLevel::Warning,
                ERK_LOG_INFO => LogLevel::Info,
                ERK_LOG_DEBUG => LogLevel::Debug,
                _ => return Err(ERK_ERR_INVALID_ARGUMENT),
            },
            system_fonts: flags & ERK_APP_EMBEDDED_FONTS == 0,
            gpu: flags & ERK_APP_CPU == 0,
        };
        let mut app = if flags & ERK_APP_HEADLESS != 0 {
            App::headless(config)
        } else {
            App::new(config)
        }
        .abi()?;
        let handle = app.handle();
        let erk_app = Box::into_raw(Box::new(ErkApp {
            thread: std::thread::current().id(),
            poisoned: AtomicBool::new(false),
            current: Cell::new(None),
            handle,
            pending: Arc::default(),
            // Set below, once the callbacks know the app's address.
            app: UnsafeCell::new(None),
        }));
        // SAFETY: just made; not shared yet.
        let shared = unsafe { &*erk_app };
        if let Some(log) = log {
            let user_data = Posted(log_user_data, erk_app);
            app.set_log(move |level, message| {
                // SAFETY: the host gave the function and its user_data.
                unsafe { log(user_data.0, level as u32, lent(message)) };
            });
        }
        if let Some(resource) = resource {
            let pending = shared.pending.clone();
            let user_data = Posted(resource_user_data, erk_app);
            app.set_resource_provider(move |cx, request, responder| {
                if let Ok(mut pending) = pending.lock() {
                    pending.insert(request.id, responder);
                }
                let kind = match request.kind {
                    ResourceKind::Image => ERK_RESOURCE_IMAGE,
                    ResourceKind::Stylesheet => ERK_RESOURCE_STYLESHEET,
                    ResourceKind::Font => ERK_RESOURCE_FONT,
                };
                // SAFETY: the app outlives its provider.
                let shared = unsafe { &*user_data.1 };
                shared.calling(cx, || {
                    // SAFETY: the host gave the function and its user_data.
                    unsafe {
                        resource(
                            user_data.0,
                            user_data.1,
                            request.id,
                            kind,
                            lent(&request.url),
                        )
                    };
                });
            });
        }
        // SAFETY: not shared yet; no callback can run.
        unsafe { *shared.app.get() = Some(app) };
        put(out, erk_app)
    })
}

/// End the app: every subscription's destroy runs once, and its ids go
/// stale. Not from inside a callback; works on a poisoned app.
#[allow(unsafe_code)] // SAFETY: an exported symbol; takes back the box erk_app_create made.
#[unsafe(no_mangle)]
pub extern "C" fn erk_app_destroy(app: *mut ErkApp) -> ErkStatus {
    if app.is_null() {
        return ERK_ERR_INVALID_ARGUMENT;
    }
    // SAFETY: as in `guard`: a live app; only the thread and callback state
    // are read before deciding.
    let shared = unsafe { &*app };
    if shared.thread != std::thread::current().id() {
        return ERK_ERR_WRONG_THREAD;
    }
    if shared.current.get().is_some() {
        return ERK_ERR_REENTRANT;
    }
    // SAFETY: erk_app_create made the box; the host gives it back once.
    guard_free(|| {
        drop(unsafe { Box::from_raw(app) });
        Ok(())
    })
}

/// Erk's event loop: open the window and run until it closes.
#[allow(unsafe_code)] // SAFETY: an exported symbol; the guard checks the app pointer.
#[unsafe(no_mangle)]
pub extern "C" fn erk_app_run(app: *mut ErkApp) -> ErkStatus {
    guard(app, Calls::Loop, |app| {
        app.app().run().map_err(|_| ERK_ERR_PLATFORM)
    })
}

/// Run `fn(user_data, app)` on the app's UI thread before its next frame;
/// `destroy(user_data)` after it, or when the app ends first. Any thread.
#[allow(unsafe_code)] // SAFETY: an exported symbol; reads only the thread-safe handle of the app.
#[unsafe(no_mangle)]
pub extern "C" fn erk_app_post(
    app: *mut ErkApp,
    function: ErkPostFn,
    user_data: *mut c_void,
    destroy: ErkDestroyFn,
) -> ErkStatus {
    guard_free(|| {
        let (Some(function), false) = (function, app.is_null()) else {
            return Err(ERK_ERR_INVALID_ARGUMENT);
        };
        // SAFETY: a live app; the handle may be used from any thread.
        let handle = unsafe { &(*app).handle };
        let posted = Posted(user_data, app);
        let destroy = Destroy(destroy, user_data);
        handle
            .post(move |cx| {
                let _destroy = destroy;
                let posted = posted;
                // SAFETY: on the UI thread, the app is alive while it runs
                // its posted work.
                let shared = unsafe { &*posted.1 };
                shared.calling(cx, || {
                    // SAFETY: the host gave the function and its user_data.
                    unsafe { function(posted.0, posted.1) };
                });
            })
            .abi()
    })
}

/// Answer resource request `request`: `ERK_OK` with the bytes and their MIME
/// type (empty: from the bytes), any other status for none. Any thread; Erk
/// copies the bytes.
#[allow(unsafe_code)] // SAFETY: an exported symbol; reads the caller's bytes and the thread-safe pending table.
#[unsafe(no_mangle)]
pub extern "C" fn erk_resource_complete(
    app: *mut ErkApp,
    request: u64,
    result: ErkStatus,
    mime: ErkStr,
    data: *const u8,
    len: usize,
) -> ErkStatus {
    guard_free(|| {
        if app.is_null() || (data.is_null() && len > 0) {
            return Err(ERK_ERR_INVALID_ARGUMENT);
        }
        let mime = text(mime)?;
        // SAFETY: a live app; the pending table may be used from any thread.
        let pending = unsafe { &(*app).pending };
        let responder = pending
            .lock()
            .map_err(|_| ERK_ERR_PANIC)?
            .remove(&request)
            .ok_or(ERK_ERR_NOT_FOUND)?;
        if result == ERK_OK {
            let bytes = if len == 0 {
                Vec::new()
            } else {
                // SAFETY: the caller gives `len` readable bytes at `data`.
                unsafe { std::slice::from_raw_parts(data, len) }.to_vec()
            };
            responder.respond(&mime, bytes);
        } else {
            responder.missing();
        }
        Ok(())
    })
}

/// A turn of a headless app at `now_ns` (§7): what arrived is handled, and a
/// frame painted if something that shows has changed.
#[allow(unsafe_code)] // SAFETY: an exported symbol; the guard checks the app pointer.
#[unsafe(no_mangle)]
pub extern "C" fn erk_app_tick(app: *mut ErkApp, now_ns: u64) -> ErkStatus {
    guard(app, Calls::Loop, |app| {
        app.app().tick(now_ns);
        Ok(())
    })
}

/// Give a headless app input, as a window would.
#[allow(unsafe_code)] // SAFETY: an exported symbol; reads the caller's input within its struct_size.
#[unsafe(no_mangle)]
pub extern "C" fn erk_app_input(app: *mut ErkApp, input: *const ErkInput) -> ErkStatus {
    guard(app, Calls::Loop, |app| {
        if input.is_null() {
            return Err(ERK_ERR_INVALID_ARGUMENT);
        }
        let none = ErkStr {
            ptr: std::ptr::null(),
            len: 0,
        };
        let kind = field!(input, ErkInput, kind, 0);
        let (x, y) = (
            field!(input, ErkInput, x, 0.0),
            field!(input, ErkInput, y, 0.0),
        );
        let bits = field!(input, ErkInput, modifiers, 0);
        let modifiers = Modifiers {
            shift: bits & ERK_MOD_SHIFT != 0,
            control: bits & ERK_MOD_CONTROL != 0,
            alt: bits & ERK_MOD_ALT != 0,
            meta: bits & ERK_MOD_META != 0,
        };
        let pointer = |kind| {
            let button = match field!(input, ErkInput, button, 0) {
                ERK_BUTTON_NONE => PointerButton::None,
                ERK_BUTTON_PRIMARY => PointerButton::Primary,
                ERK_BUTTON_SECONDARY => PointerButton::Secondary,
                ERK_BUTTON_MIDDLE => PointerButton::Middle,
                _ => return Err(ERK_ERR_INVALID_ARGUMENT),
            };
            Ok(Input::Pointer(PointerInput {
                kind,
                x,
                y,
                button,
                modifiers,
            }))
        };
        let key = |state| {
            let key = match field!(input, ErkInput, key, 0) {
                ERK_KEY_OTHER => Key::Other,
                ERK_KEY_TAB => Key::Tab,
                ERK_KEY_ENTER => Key::Enter,
                ERK_KEY_SPACE => Key::Space,
                ERK_KEY_ESCAPE => Key::Escape,
                ERK_KEY_BACKSPACE => Key::Backspace,
                ERK_KEY_CHARACTER => Key::Character(text(field!(input, ErkInput, text, none))?),
                _ => return Err(ERK_ERR_INVALID_ARGUMENT),
            };
            Ok(Input::Key(KeyInput {
                key,
                state,
                modifiers,
            }))
        };
        let input = match kind {
            ERK_INPUT_POINTER_MOVE => pointer(PointerKind::Move)?,
            ERK_INPUT_POINTER_DOWN => pointer(PointerKind::Down)?,
            ERK_INPUT_POINTER_UP => pointer(PointerKind::Up)?,
            ERK_INPUT_POINTER_LEAVE => pointer(PointerKind::Leave)?,
            ERK_INPUT_KEY_DOWN => key(KeyState::Down)?,
            ERK_INPUT_KEY_UP => key(KeyState::Up)?,
            ERK_INPUT_WHEEL => Input::Wheel {
                dx: field!(input, ErkInput, dx, 0.0),
                dy: field!(input, ErkInput, dy, 0.0),
                x,
                y,
            },
            _ => return Err(ERK_ERR_INVALID_ARGUMENT),
        };
        app.app().input(input);
        Ok(())
    })
}

/// A headless app's last frame; `ERK_ERR_NOT_FOUND` before the first.
#[allow(unsafe_code)] // SAFETY: an exported symbol; writes the caller's frame within its struct_size.
#[unsafe(no_mangle)]
pub extern "C" fn erk_app_frame(app: *mut ErkApp, out: *mut ErkFrame) -> ErkStatus {
    guard(app, Calls::Loop, |app| {
        let frame = app.app().frame().ok_or(ERK_ERR_NOT_FOUND)?;
        put_sized(
            out,
            ErkFrame {
                struct_size: 0,
                width: u32::from(frame.width()),
                height: u32::from(frame.height()),
                rgba: frame.rgba().as_ptr(),
                len: frame.rgba().len(),
            },
        )
    })
}

/// Show `html` instead of the document; the old ids go stale.
#[allow(unsafe_code)] // SAFETY: an exported symbol; the guard checks the app pointer.
#[unsafe(no_mangle)]
pub extern "C" fn erk_load_html(app: *mut ErkApp, html: ErkStr) -> ErkStatus {
    guard(app, Calls::Document, |app| {
        app.cx().load_html(&text(html)?);
        Ok(())
    })
}

#[allow(unsafe_code)] // SAFETY: an exported symbol; the guard checks the app pointer.
#[unsafe(no_mangle)]
pub extern "C" fn erk_document_root(app: *mut ErkApp, out: *mut ErkNodeId) -> ErkStatus {
    guard(app, Calls::Document, |app| {
        put(out, app.cx().root().to_raw())
    })
}

/// The first element inside `scope` (`ERK_NODE_NONE`: the document)
/// matching `selector`; `*out` is `ERK_NODE_NONE` when none does.
#[allow(unsafe_code)] // SAFETY: an exported symbol; the guard checks the app pointer.
#[unsafe(no_mangle)]
pub extern "C" fn erk_query(
    app: *mut ErkApp,
    scope: ErkNodeId,
    selector: ErkStr,
    out: *mut ErkNodeId,
) -> ErkStatus {
    guard(app, Calls::Document, |app| {
        let scope = (scope != ERK_NODE_NONE).then(|| node(scope)).transpose()?;
        let found = app.cx().query(scope, &text(selector)?).abi()?;
        put(out, raw(found))
    })
}

/// Open a transaction: until the outermost one is committed no frame is
/// prepared, so none shows the document halfway through a group of
/// changes (p1-contract §10, v0.6). Transactions nest.
#[allow(unsafe_code)] // SAFETY: an exported symbol; the guard checks the app pointer.
#[unsafe(no_mangle)]
pub extern "C" fn erk_transaction_begin(app: *mut ErkApp) -> ErkStatus {
    guard(app, Calls::Document, |app| {
        app.cx().begin_transaction();
        Ok(())
    })
}

/// Close the innermost transaction; `ERK_ERR_INVALID_ARGUMENT` if none is
/// open.
#[allow(unsafe_code)] // SAFETY: an exported symbol; the guard checks the app pointer.
#[unsafe(no_mangle)]
pub extern "C" fn erk_transaction_commit(app: *mut ErkApp) -> ErkStatus {
    guard(app, Calls::Document, |app| {
        app.cx().commit_transaction().abi()
    })
}

#[allow(unsafe_code)] // SAFETY: an exported symbol; the guard checks the app pointer.
#[unsafe(no_mangle)]
pub extern "C" fn erk_node_set_text(
    app: *mut ErkApp,
    node_id: ErkNodeId,
    value: ErkStr,
) -> ErkStatus {
    guard(app, Calls::Document, |app| {
        app.cx().set_text(node(node_id)?, &text(value)?).abi()
    })
}

/// `node`'s text as `textContent`; `*len` is the bytes needed (no NUL).
#[allow(unsafe_code)] // SAFETY: an exported symbol; the guard checks the app pointer.
#[unsafe(no_mangle)]
pub extern "C" fn erk_node_text(
    app: *mut ErkApp,
    node_id: ErkNodeId,
    buf: *mut c_char,
    cap: usize,
    len: *mut usize,
) -> ErkStatus {
    guard(app, Calls::Document, |app| {
        let value = app.cx().text(node(node_id)?).abi()?;
        put_text(&value, buf, cap, len)
    })
}

/// A new element named `tag`, not in the document yet: insert it with
/// `erk_node_append` or `erk_node_insert_before`, or let it go with
/// `erk_node_remove`. HTML lowercases the name.
#[allow(unsafe_code)] // SAFETY: an exported symbol; the guard checks the app pointer.
#[unsafe(no_mangle)]
pub extern "C" fn erk_node_create(app: *mut ErkApp, tag: ErkStr, out: *mut ErkNodeId) -> ErkStatus {
    guard(app, Calls::Document, |app| {
        let made = app.cx().create_element(&text(tag)?).abi()?;
        put(out, made.to_raw())
    })
}

/// A new text node, not in the document yet.
#[allow(unsafe_code)] // SAFETY: an exported symbol; the guard checks the app pointer.
#[unsafe(no_mangle)]
pub extern "C" fn erk_text_create(
    app: *mut ErkApp,
    value: ErkStr,
    out: *mut ErkNodeId,
) -> ErkStatus {
    guard(app, Calls::Document, |app| {
        let made = app.cx().create_text(&text(value)?);
        put(out, made.to_raw())
    })
}

/// Make `child` the last child of `parent`, moving it from wherever it
/// was. `ERK_ERR_INVALID_ARGUMENT` where DOM refuses it.
#[allow(unsafe_code)] // SAFETY: an exported symbol; the guard checks the app pointer.
#[unsafe(no_mangle)]
pub extern "C" fn erk_node_append(
    app: *mut ErkApp,
    parent: ErkNodeId,
    child: ErkNodeId,
) -> ErkStatus {
    guard(app, Calls::Document, |app| {
        app.cx().append(node(parent)?, node(child)?).abi()
    })
}

/// Insert `child` into `parent` before `before`, a child of `parent`, or
/// last for `ERK_NODE_NONE`.
#[allow(unsafe_code)] // SAFETY: an exported symbol; the guard checks the app pointer.
#[unsafe(no_mangle)]
pub extern "C" fn erk_node_insert_before(
    app: *mut ErkApp,
    parent: ErkNodeId,
    child: ErkNodeId,
    before: ErkNodeId,
) -> ErkStatus {
    guard(app, Calls::Document, |app| {
        let before = (before != ERK_NODE_NONE)
            .then(|| node(before))
            .transpose()?;
        app.cx()
            .insert_before(node(parent)?, node(child)?, before)
            .abi()
    })
}

/// Remove `node` and everything in it: their ids go stale and their
/// subscriptions end.
#[allow(unsafe_code)] // SAFETY: an exported symbol; the guard checks the app pointer.
#[unsafe(no_mangle)]
pub extern "C" fn erk_node_remove(app: *mut ErkApp, node_id: ErkNodeId) -> ErkStatus {
    guard(app, Calls::Document, |app| {
        app.cx().remove(node(node_id)?).abi()
    })
}

#[allow(unsafe_code)] // SAFETY: an exported symbol; the guard checks the app pointer.
#[unsafe(no_mangle)]
pub extern "C" fn erk_node_set_attr(
    app: *mut ErkApp,
    node_id: ErkNodeId,
    name: ErkStr,
    value: ErkStr,
) -> ErkStatus {
    guard(app, Calls::Document, |app| {
        app.cx()
            .set_attr(node(node_id)?, &text(name)?, &text(value)?)
            .abi()
    })
}

/// Remove attribute `name`; `ERK_ERR_NOT_FOUND` if there was none.
#[allow(unsafe_code)] // SAFETY: an exported symbol; the guard checks the app pointer.
#[unsafe(no_mangle)]
pub extern "C" fn erk_node_remove_attr(
    app: *mut ErkApp,
    node_id: ErkNodeId,
    name: ErkStr,
) -> ErkStatus {
    guard(app, Calls::Document, |app| {
        match app.cx().remove_attr(node(node_id)?, &text(name)?).abi()? {
            true => Ok(()),
            false => Err(ERK_ERR_NOT_FOUND),
        }
    })
}

/// Attribute `name`'s value; `ERK_ERR_NOT_FOUND` without one.
#[allow(unsafe_code)] // SAFETY: an exported symbol; the guard checks the app pointer.
#[unsafe(no_mangle)]
pub extern "C" fn erk_node_attr(
    app: *mut ErkApp,
    node_id: ErkNodeId,
    name: ErkStr,
    buf: *mut c_char,
    cap: usize,
    len: *mut usize,
) -> ErkStatus {
    guard(app, Calls::Document, |app| {
        let value = app
            .cx()
            .attr(node(node_id)?, &text(name)?)
            .abi()?
            .ok_or(ERK_ERR_NOT_FOUND)?;
        put_text(&value, buf, cap, len)
    })
}

#[allow(unsafe_code)] // SAFETY: an exported symbol; the guard checks the app pointer.
#[unsafe(no_mangle)]
pub extern "C" fn erk_node_add_class(
    app: *mut ErkApp,
    node_id: ErkNodeId,
    class: ErkStr,
) -> ErkStatus {
    guard(app, Calls::Document, |app| {
        app.cx().add_class(node(node_id)?, &text(class)?).abi()
    })
}

#[allow(unsafe_code)] // SAFETY: an exported symbol; the guard checks the app pointer.
#[unsafe(no_mangle)]
pub extern "C" fn erk_node_remove_class(
    app: *mut ErkApp,
    node_id: ErkNodeId,
    class: ErkStr,
) -> ErkStatus {
    guard(app, Calls::Document, |app| {
        app.cx().remove_class(node(node_id)?, &text(class)?).abi()
    })
}

/// `*out` is 1 if element `node`'s classes hold `class`, else 0.
#[allow(unsafe_code)] // SAFETY: an exported symbol; the guard checks the app pointer.
#[unsafe(no_mangle)]
pub extern "C" fn erk_node_has_class(
    app: *mut ErkApp,
    node_id: ErkNodeId,
    class: ErkStr,
    out: *mut u32,
) -> ErkStatus {
    guard(app, Calls::Document, |app| {
        let has = app.cx().has_class(node(node_id)?, &text(class)?).abi()?;
        put(out, u32::from(has))
    })
}

/// Every element inside `scope` (`ERK_NODE_NONE`: the document) matching
/// `selector`, in document order, into `out`: all of them or none.
/// `*len` is always how many there are; `ERK_ERR_BUFFER_TOO_SMALL` when
/// `cap` is less.
#[allow(unsafe_code)] // SAFETY: an exported symbol; writes at most `cap` ids into the caller's array.
#[unsafe(no_mangle)]
pub extern "C" fn erk_query_all(
    app: *mut ErkApp,
    scope: ErkNodeId,
    selector: ErkStr,
    out: *mut ErkNodeId,
    cap: usize,
    len: *mut usize,
) -> ErkStatus {
    guard(app, Calls::Document, |app| {
        let scope = (scope != ERK_NODE_NONE).then(|| node(scope)).transpose()?;
        let found = app.cx().query_all(scope, &text(selector)?).abi()?;
        put(len, found.len())?;
        if cap < found.len() {
            return Err(ERK_ERR_BUFFER_TOO_SMALL);
        }
        if found.is_empty() {
            return Ok(());
        }
        if out.is_null() {
            return Err(ERK_ERR_INVALID_ARGUMENT);
        }
        for (at, node) in found.iter().enumerate() {
            // SAFETY: the caller gives `cap` writable ids at `out`, and `at`
            // is below `found.len()`, at most `cap`.
            unsafe { out.add(at).write(node.to_raw()) };
        }
        Ok(())
    })
}

/// A node id or a reference to what an earlier mutation created.
fn reference(id: ErkNodeId) -> Result<Ref, ErkStatus> {
    // No generation in the upper half: not a node id, a batch position.
    if id != ERK_NODE_NONE && id >> 32 == 0 {
        let at = usize::try_from(id - ERK_NEW_NODE).map_err(|_| ERK_ERR_INVALID_ARGUMENT)?;
        return Ok(Ref::New(at));
    }
    Ok(Ref::Node(node(id)?))
}

/// The mutation `raw` describes, its fields read within its struct_size.
fn mutation(raw: *const ErkMutation) -> Result<Mutation, ErkStatus> {
    let none = ErkStr {
        ptr: std::ptr::null(),
        len: 0,
    };
    let kind = field!(raw, ErkMutation, kind, 0);
    let target = || reference(field!(raw, ErkMutation, node, ERK_NODE_NONE));
    let parent = || reference(field!(raw, ErkMutation, parent, ERK_NODE_NONE));
    let name = || text(field!(raw, ErkMutation, name, none));
    let value = || text(field!(raw, ErkMutation, value, none));
    Ok(match kind {
        ERK_MUTATION_CREATE_ELEMENT => Mutation::CreateElement(value()?),
        ERK_MUTATION_CREATE_TEXT => Mutation::CreateText(value()?),
        ERK_MUTATION_APPEND => Mutation::Append {
            parent: parent()?,
            child: target()?,
        },
        ERK_MUTATION_INSERT_BEFORE => {
            let before = field!(raw, ErkMutation, before, ERK_NODE_NONE);
            Mutation::InsertBefore {
                parent: parent()?,
                child: target()?,
                before: (before != ERK_NODE_NONE)
                    .then(|| reference(before))
                    .transpose()?,
            }
        }
        ERK_MUTATION_REMOVE => Mutation::Remove(target()?),
        ERK_MUTATION_SET_TEXT => Mutation::SetText(target()?, value()?),
        ERK_MUTATION_SET_ATTR => Mutation::SetAttr(target()?, name()?, value()?),
        ERK_MUTATION_REMOVE_ATTR => Mutation::RemoveAttr(target()?, name()?),
        ERK_MUTATION_ADD_CLASS => Mutation::AddClass(target()?, value()?),
        ERK_MUTATION_REMOVE_CLASS => Mutation::RemoveClass(target()?, value()?),
        _ => return Err(ERK_ERR_INVALID_ARGUMENT),
    })
}

/// Apply `count` mutations in order, in one call (p1-contract §10). The ids
/// the batch created go to `created` (`count` entries, `ERK_NODE_NONE` for
/// mutations that create nothing; may be NULL). The first that fails stops
/// the batch: its position goes to `*failed_at` (may be NULL) and its
/// status is returned; the ones before it stay applied.
#[allow(unsafe_code)] // SAFETY: an exported symbol; reads `count` mutations and writes `count` ids.
#[unsafe(no_mangle)]
pub extern "C" fn erk_apply(
    app: *mut ErkApp,
    mutations: *const ErkMutation,
    count: usize,
    created: *mut ErkNodeId,
    failed_at: *mut usize,
) -> ErkStatus {
    guard(app, Calls::Document, |app| {
        if count == 0 {
            return Ok(());
        }
        if mutations.is_null() {
            return Err(ERK_ERR_INVALID_ARGUMENT);
        }
        // SAFETY: every element starts with its struct_size, the array's
        // stride.
        let stride = unsafe { (*mutations).struct_size } as usize;
        if stride < offset_of!(ErkMutation, node) {
            return Err(ERK_ERR_INVALID_ARGUMENT);
        }
        let report = |at: usize, status: ErkStatus| -> ErkStatus {
            if !failed_at.is_null() {
                // SAFETY: the caller gives a writable size_t when not NULL.
                unsafe { failed_at.write(at) };
            }
            status
        };
        let mut batch = Vec::with_capacity(count);
        for at in 0..count {
            // SAFETY: the caller gives `count` mutations `stride` bytes
            // apart.
            let raw = unsafe { mutations.cast::<u8>().add(at * stride) }.cast::<ErkMutation>();
            batch.push(mutation(raw).map_err(|status| report(at, status))?);
        }
        let made = match app.cx().apply(&batch) {
            Ok(made) => made,
            Err(failed) => return Err(report(failed.index, status(failed.status))),
        };
        if !created.is_null() {
            for (at, node) in made.into_iter().enumerate() {
                // SAFETY: the caller gives `count` writable ids at `created`.
                unsafe { created.add(at).write(raw(node)) };
            }
        }
        Ok(())
    })
}

/// The `ERK_MOD_*` bits of `modifiers`.
fn modifier_bits(modifiers: Modifiers) -> u32 {
    [
        (modifiers.shift, ERK_MOD_SHIFT),
        (modifiers.control, ERK_MOD_CONTROL),
        (modifiers.alt, ERK_MOD_ALT),
        (modifiers.meta, ERK_MOD_META),
    ]
    .into_iter()
    .filter(|(held, _)| *held)
    .fold(0, |bits, (_, bit)| bits | bit)
}

fn kind_of(kind: u32) -> Result<EventKind, ErkStatus> {
    match kind {
        ERK_EVENT_CLICK => Ok(EventKind::Click),
        ERK_EVENT_KEY_DOWN => Ok(EventKind::KeyDown),
        ERK_EVENT_KEY_UP => Ok(EventKind::KeyUp),
        ERK_EVENT_FOCUS => Ok(EventKind::Focus),
        ERK_EVENT_BLUR => Ok(EventKind::Blur),
        // Input, change and submit come with forms (M5).
        _ => Err(ERK_ERR_INVALID_ARGUMENT),
    }
}

/// The host's event callback: the function, its data and its destroy.
struct HostCallback {
    function: ErkEventFn,
    user_data: *mut c_void,
    destroy: ErkDestroyFn,
}

fn subscribe(
    app: *mut ErkApp,
    node_id: ErkNodeId,
    kind: u32,
    capture: bool,
    callback: HostCallback,
    out: *mut ErkSubscription,
) -> ErkStatus {
    let HostCallback {
        function,
        user_data,
        destroy,
    } = callback;
    guard(app, Calls::Document, move |shared| {
        let Some(function) = function else {
            return Err(ERK_ERR_INVALID_ARGUMENT);
        };
        if out.is_null() {
            return Err(ERK_ERR_INVALID_ARGUMENT);
        }
        let (node, kind) = (node(node_id)?, kind_of(kind)?);
        // A stale node fails here, before the subscription owns user_data:
        // a call that fails leaves it, and its destroy, with the host.
        shared.cx().kind(node).abi()?;
        let destroy = Destroy(destroy, user_data);
        let target = Posted(user_data, app);
        let callback = move |cx: &mut Context, event: &Event| {
            let _ = &destroy;
            let event = ErkEvent {
                struct_size: size_of::<ErkEvent>() as u32,
                kind: event.kind as u32,
                phase: match event.phase {
                    Phase::Capture => ERK_PHASE_CAPTURE,
                    Phase::Target => ERK_PHASE_TARGET,
                    Phase::Bubble => ERK_PHASE_BUBBLE,
                },
                target: event.target.to_raw(),
                current_target: event.current_target.to_raw(),
                x: f64::from(event.x),
                y: f64::from(event.y),
                modifiers: modifier_bits(event.modifiers),
                text: match &event.key {
                    Some(Key::Character(typed)) => lent(typed),
                    _ => lent(""),
                },
                key: match &event.key {
                    None | Some(Key::Other) => ERK_KEY_OTHER,
                    Some(Key::Tab) => ERK_KEY_TAB,
                    Some(Key::Enter) => ERK_KEY_ENTER,
                    Some(Key::Space) => ERK_KEY_SPACE,
                    Some(Key::Escape) => ERK_KEY_ESCAPE,
                    Some(Key::Backspace) => ERK_KEY_BACKSPACE,
                    Some(Key::Character(_)) => ERK_KEY_CHARACTER,
                },
            };
            #[allow(unsafe_code)] // SAFETY: the app outlives its subscriptions.
            let shared = unsafe { &*target.1 };
            shared.calling(cx, || {
                #[allow(unsafe_code)] // SAFETY: the host gave the function and its user_data.
                unsafe {
                    function(target.0, target.1, &event)
                };
            });
        };
        let cx = shared.cx();
        let subscription = if capture {
            cx.on_capture(node, kind, callback)
        } else {
            cx.on(node, kind, callback)
        }
        .abi()?;
        put(out, subscription.to_raw())
    })
}

/// Call `fn(user_data, app, event)` when an event of `kind` reaches `node`
/// at its target or bubbling up; `destroy(user_data)` once, when the
/// subscription ends (`erk_off`, the node leaving the document, the app's
/// end). A call that fails takes nothing: `user_data` stays the host's.
#[allow(unsafe_code)] // SAFETY: an exported symbol; the guard checks the app pointer.
#[unsafe(no_mangle)]
pub extern "C" fn erk_on(
    app: *mut ErkApp,
    node_id: ErkNodeId,
    kind: u32,
    function: ErkEventFn,
    user_data: *mut c_void,
    destroy: ErkDestroyFn,
    out: *mut ErkSubscription,
) -> ErkStatus {
    let callback = HostCallback {
        function,
        user_data,
        destroy,
    };
    subscribe(app, node_id, kind, false, callback, out)
}

/// `erk_on` in the capture phase: on the way down, and at the target before
/// the bubble subscriptions there.
#[allow(unsafe_code)] // SAFETY: an exported symbol; the guard checks the app pointer.
#[unsafe(no_mangle)]
pub extern "C" fn erk_on_capture(
    app: *mut ErkApp,
    node_id: ErkNodeId,
    kind: u32,
    function: ErkEventFn,
    user_data: *mut c_void,
    destroy: ErkDestroyFn,
    out: *mut ErkSubscription,
) -> ErkStatus {
    let callback = HostCallback {
        function,
        user_data,
        destroy,
    };
    subscribe(app, node_id, kind, true, callback, out)
}

#[allow(unsafe_code)] // SAFETY: an exported symbol; the guard checks the app pointer.
#[unsafe(no_mangle)]
pub extern "C" fn erk_off(app: *mut ErkApp, subscription: ErkSubscription) -> ErkStatus {
    guard(app, Calls::Document, |app| {
        let subscription = Subscription::from_raw(subscription).ok_or(ERK_ERR_INVALID_ARGUMENT)?;
        app.cx().off(subscription).abi()
    })
}

/// Inside an event callback: the event goes no further than this node.
#[allow(unsafe_code)] // SAFETY: an exported symbol; the guard checks the app pointer.
#[unsafe(no_mangle)]
pub extern "C" fn erk_event_stop_propagation(app: *mut ErkApp) -> ErkStatus {
    guard(app, Calls::Document, |app| {
        if app.current.get().is_none() {
            return Err(ERK_ERR_INVALID_ARGUMENT);
        }
        app.cx().stop_propagation();
        Ok(())
    })
}

#[allow(unsafe_code)] // SAFETY: an exported symbol; the guard checks the app pointer.
#[unsafe(no_mangle)]
pub extern "C" fn erk_node_parent(
    app: *mut ErkApp,
    node_id: ErkNodeId,
    out: *mut ErkNodeId,
) -> ErkStatus {
    guard(app, Calls::Document, |app| {
        let parent = app.cx().parent(node(node_id)?).abi()?;
        put(out, raw(parent))
    })
}

/// `node`'s child at `index`; `*out` is `ERK_NODE_NONE` past the last.
#[allow(unsafe_code)] // SAFETY: an exported symbol; the guard checks the app pointer.
#[unsafe(no_mangle)]
pub extern "C" fn erk_node_child_at(
    app: *mut ErkApp,
    node_id: ErkNodeId,
    index: usize,
    out: *mut ErkNodeId,
) -> ErkStatus {
    guard(app, Calls::Document, |app| {
        let child = app.cx().child_at(node(node_id)?, index).abi()?;
        put(out, raw(child))
    })
}

#[allow(unsafe_code)] // SAFETY: an exported symbol; the guard checks the app pointer.
#[unsafe(no_mangle)]
pub extern "C" fn erk_node_child_count(
    app: *mut ErkApp,
    node_id: ErkNodeId,
    out: *mut usize,
) -> ErkStatus {
    guard(app, Calls::Document, |app| {
        put(out, app.cx().child_count(node(node_id)?).abi()?)
    })
}

/// `ERK_NODE_*`.
#[allow(unsafe_code)] // SAFETY: an exported symbol; the guard checks the app pointer.
#[unsafe(no_mangle)]
pub extern "C" fn erk_node_kind(app: *mut ErkApp, node_id: ErkNodeId, out: *mut u32) -> ErkStatus {
    guard(app, Calls::Document, |app| {
        let kind = match app.cx().kind(node(node_id)?).abi()? {
            NodeKind::Document => ERK_NODE_DOCUMENT,
            NodeKind::Element => ERK_NODE_ELEMENT,
            NodeKind::Text => ERK_NODE_TEXT,
            NodeKind::Comment => ERK_NODE_COMMENT,
            NodeKind::Other => ERK_NODE_OTHER,
        };
        put(out, kind)
    })
}

/// An element's tag name; `ERK_ERR_NOT_FOUND` for other nodes.
#[allow(unsafe_code)] // SAFETY: an exported symbol; the guard checks the app pointer.
#[unsafe(no_mangle)]
pub extern "C" fn erk_node_tag(
    app: *mut ErkApp,
    node_id: ErkNodeId,
    buf: *mut c_char,
    cap: usize,
    len: *mut usize,
) -> ErkStatus {
    guard(app, Calls::Document, |app| {
        let tag = app
            .cx()
            .tag(node(node_id)?)
            .abi()?
            .ok_or(ERK_ERR_NOT_FOUND)?;
        put_text(&tag, buf, cap, len)
    })
}

#[allow(unsafe_code)] // SAFETY: an exported symbol; the guard checks the app pointer.
#[unsafe(no_mangle)]
pub extern "C" fn erk_node_attribute_count(
    app: *mut ErkApp,
    node_id: ErkNodeId,
    out: *mut usize,
) -> ErkStatus {
    guard(app, Calls::Document, |app| {
        put(out, app.cx().attributes(node(node_id)?).abi()?.len())
    })
}

/// An element's attribute at `index`, in the order written; free both
/// strings. `ERK_ERR_NOT_FOUND` past the last.
#[allow(unsafe_code)] // SAFETY: an exported symbol; the guard checks the app pointer.
#[unsafe(no_mangle)]
pub extern "C" fn erk_node_attribute_at(
    app: *mut ErkApp,
    node_id: ErkNodeId,
    index: usize,
    name: *mut ErkString,
    value: *mut ErkString,
) -> ErkStatus {
    guard(app, Calls::Document, |app| {
        if name.is_null() || value.is_null() {
            return Err(ERK_ERR_INVALID_ARGUMENT);
        }
        let (found_name, found_value) = app
            .cx()
            .attributes(node(node_id)?)
            .abi()?
            .into_iter()
            .nth(index)
            .ok_or(ERK_ERR_NOT_FOUND)?;
        put(name, owned(found_name))?;
        put(value, owned(found_value))
    })
}

/// `node`'s box in the last frame; `ERK_ERR_NOT_FOUND` without one.
#[allow(unsafe_code)] // SAFETY: an exported symbol; the guard checks the app pointer.
#[unsafe(no_mangle)]
pub extern "C" fn erk_node_box(
    app: *mut ErkApp,
    node_id: ErkNodeId,
    out: *mut ErkBox,
) -> ErkStatus {
    guard(app, Calls::Document, |app| {
        let found = app.cx().node_box(node(node_id)?).abi()?;
        put_sized(
            out,
            ErkBox {
                struct_size: 0,
                x: found.x,
                y: found.y,
                width: found.width,
                height: found.height,
                margin: found.margin,
                border: found.border,
                padding: found.padding,
            },
        )
    })
}

/// `node`'s computed style as `name: value;` lines; free the string.
#[allow(unsafe_code)] // SAFETY: an exported symbol; the guard checks the app pointer.
#[unsafe(no_mangle)]
pub extern "C" fn erk_node_computed_style(
    app: *mut ErkApp,
    node_id: ErkNodeId,
    out: *mut ErkString,
) -> ErkStatus {
    guard(app, Calls::Document, |app| {
        let style = app.cx().computed_style(node(node_id)?).abi()?;
        put(out, owned(style))
    })
}

/// The topmost node at `x`, `y` in the last frame; `ERK_NODE_NONE` if none.
#[allow(unsafe_code)] // SAFETY: an exported symbol; the guard checks the app pointer.
#[unsafe(no_mangle)]
pub extern "C" fn erk_inspect_at(
    app: *mut ErkApp,
    x: f32,
    y: f32,
    out: *mut ErkNodeId,
) -> ErkStatus {
    guard(app, Calls::Document, |app| {
        put(out, raw(app.cx().inspect_at(x, y)))
    })
}

/// Highlight `node`'s boxes over the page; `ERK_NODE_NONE` clears.
#[allow(unsafe_code)] // SAFETY: an exported symbol; the guard checks the app pointer.
#[unsafe(no_mangle)]
pub extern "C" fn erk_highlight(app: *mut ErkApp, node_id: ErkNodeId) -> ErkStatus {
    guard(app, Calls::Document, |app| {
        let target = (node_id != ERK_NODE_NONE)
            .then(|| node(node_id))
            .transpose()?;
        app.cx().highlight(target).abi()
    })
}

/// How long the last painted frame's stages took; `ERK_ERR_NOT_FOUND`
/// before the first.
#[allow(unsafe_code)] // SAFETY: an exported symbol; the guard checks the app pointer.
#[unsafe(no_mangle)]
pub extern "C" fn erk_last_frame_timings(app: *mut ErkApp, out: *mut ErkFrameTimings) -> ErkStatus {
    guard(app, Calls::Loop, |app| {
        let timings = app.app().last_frame_timings().ok_or(ERK_ERR_NOT_FOUND)?;
        put_sized(
            out,
            ErkFrameTimings {
                struct_size: 0,
                frame: timings.frame,
                style_ns: timings.style_ns,
                layout_ns: timings.layout_ns,
                display_list_ns: timings.display_list_ns,
                raster_ns: timings.raster_ns,
            },
        )
    })
}

#[cfg(test)]
mod tests;
