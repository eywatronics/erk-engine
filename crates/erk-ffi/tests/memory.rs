//! Ten thousand create/remove cycles leave the heap where they found it
//! (M4.2, M4 acceptance). A host that adds and removes rows all day must
//! not grow: every slot, side table, subscription and raster resource a
//! node took comes back when it goes.
//!
//! The count is of live heap bytes, by a counting global allocator in this
//! test binary: every thread's allocations, the engine's frame thread
//! included. Counting the arena and the side tables instead would miss
//! what they do not know about (Stylo's data, text layouts, the raster's
//! tables, the C ABI's callbacks). It lives in this crate because a global
//! allocator is unsafe code, which only the listed crates may have.

use std::alloc::{GlobalAlloc, Layout, System};
use std::ffi::c_void;
use std::sync::atomic::{AtomicIsize, Ordering};

use erk_ffi::*;

/// The system allocator, counting the bytes it has handed out and not got
/// back.
struct Counting;

static LIVE: AtomicIsize = AtomicIsize::new(0);

fn count(bytes: usize, sign: isize) {
    LIVE.fetch_add(sign * bytes as isize, Ordering::Relaxed);
}

#[allow(unsafe_code)] // SAFETY: forwards each call to the system allocator unchanged and only counts.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller keeps GlobalAlloc's contract, which is System's.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            count(layout.size(), 1);
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: as above.
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            count(layout.size(), 1);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: as above; `ptr` came from System through this allocator.
        unsafe { System.dealloc(ptr, layout) };
        count(layout.size(), -1);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: as above.
        let new = unsafe { System.realloc(ptr, layout, new_size) };
        if !new.is_null() {
            count(layout.size(), -1);
            count(new_size, 1);
        }
        new
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn s(text: &str) -> ErkStr {
    ErkStr {
        ptr: text.as_ptr().cast(),
        len: text.len(),
    }
}

#[allow(unsafe_code)] // SAFETY: a C callback that touches nothing.
unsafe extern "C" fn clicked(_: *mut c_void, _: *mut ErkApp, _: *const ErkEvent) {}

#[allow(unsafe_code)] // SAFETY: a C callback that touches nothing.
unsafe extern "C" fn destroyed(_: *mut c_void) {}

/// One row as a host makes it: an element with text, attributes, a class
/// and a subscription, in the page, painted, hovered, clicked and removed.
/// The next cycle's frame is the first without it.
fn cycle(app: *mut ErkApp, list: ErkNodeId, now: &mut u64) {
    let (mut row, mut text, mut subscription) = (0, 0, 0);
    assert_eq!(erk_node_create(app, s("li"), &mut row), ERK_OK);
    assert_eq!(erk_text_create(app, s("bir görev"), &mut text), ERK_OK);
    assert_eq!(erk_node_append(app, row, text), ERK_OK);
    assert_eq!(erk_node_set_attr(app, row, s("data-id"), s("42")), ERK_OK);
    assert_eq!(
        erk_node_set_attr(app, row, s("style"), s("height: 20px; background: #eee")),
        ERK_OK
    );
    assert_eq!(erk_node_add_class(app, row, s("yeni")), ERK_OK);
    assert_eq!(
        erk_on(
            app,
            row,
            ERK_EVENT_CLICK,
            Some(clicked),
            std::ptr::null_mut(),
            Some(destroyed),
            &mut subscription
        ),
        ERK_OK
    );
    assert_eq!(erk_node_append(app, list, row), ERK_OK);
    *now += 16_000_000;
    assert_eq!(erk_app_tick(app, *now), ERK_OK);
    for kind in [
        ERK_INPUT_POINTER_MOVE,
        ERK_INPUT_POINTER_DOWN,
        ERK_INPUT_POINTER_UP,
    ] {
        let input = ErkInput {
            struct_size: size_of::<ErkInput>() as u32,
            kind,
            x: 10.0,
            y: 10.0,
            dx: 0.0,
            dy: 0.0,
            button: ERK_BUTTON_PRIMARY,
            key: 0,
            text: s(""),
            modifiers: 0,
        };
        assert_eq!(erk_app_input(app, &input), ERK_OK);
    }
    assert_eq!(erk_node_remove(app, row), ERK_OK);
}

#[test]
fn ten_thousand_rows_made_and_removed_leave_the_heap_as_it_was() {
    let config = ErkConfig {
        struct_size: size_of::<ErkConfig>() as u32,
        width: 64,
        height: 48,
        scale: 0.0,
        title: s(""),
        resource: None,
        resource_user_data: std::ptr::null_mut(),
        log: None,
        log_user_data: std::ptr::null_mut(),
        log_level: 0,
        flags: ERK_APP_HEADLESS | ERK_APP_EMBEDDED_FONTS,
    };
    let mut app = std::ptr::null_mut();
    assert_eq!(erk_app_create(&config, &mut app), ERK_OK);
    let page = "<body style='margin: 0'><ul id=list style='margin: 0; padding: 0'></ul>";
    assert_eq!(erk_load_html(app, s(page)), ERK_OK);
    let mut list = 0;
    assert_eq!(erk_query(app, ERK_NODE_NONE, s("#list"), &mut list), ERK_OK);
    let mut now = 0;
    // Warm up: caches and tables reach their working size.
    for _ in 0..200 {
        cycle(app, list, &mut now);
    }
    let before = LIVE.load(Ordering::SeqCst);
    for _ in 0..10_000 {
        cycle(app, list, &mut now);
    }
    let grown = LIVE.load(Ordering::SeqCst) - before;
    eprintln!("heap growth over 10 000 cycles: {grown} bytes");
    // A leak of 8 bytes a cycle would be 80 000.
    assert!(
        grown < 16 * 1024,
        "the heap grew by {grown} bytes over 10 000 cycles"
    );
    assert_eq!(erk_app_destroy(app), ERK_OK);
}
