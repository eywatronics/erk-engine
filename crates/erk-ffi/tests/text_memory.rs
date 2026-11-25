//! What a paragraph keeps once it is laid out (M5.13). Parley grows a
//! layout's clusters and glyphs as it shapes, and a kept layout held room
//! it never used: on the page of 10 000 cards Parley's share of the heap
//! was 67.6 MB, 27.9 MB without that room.
//!
//! The count is of live heap bytes, by a counting global allocator in this
//! test binary, as in `memory.rs`; the one test here keeps the count to
//! itself. A card's share is the difference between two pages, so what an
//! app holds whatever its page (fonts, Stylo) drops out.

use std::alloc::{GlobalAlloc, Layout, System};
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

/// A page of `cards` cards like the card page's (`examples/erk-10000.html`):
/// a title and a sentence each.
fn cards_page(cards: usize) -> String {
    let mut page = String::from(
        "<style>body { margin: 0; font-family: 'Noto Sans' } \
         .card { margin: 8px; padding: 8px; border: 1px solid #ccc } \
         .title { font-weight: bold }</style><body>",
    );
    for card in 0..cards {
        page.push_str(&format!(
            "<div class=card><div class=title>Kart {card}</div>\
             <div class=description>Bu, Erk yerleşim benchmark testi için örnek \
             içeriktir. Kart numarası: {card}.</div></div>"
        ));
    }
    page
}

/// The live heap an app holds with `html` laid out and painted once.
fn held(html: &str) -> isize {
    let config = ErkConfig {
        struct_size: size_of::<ErkConfig>() as u32,
        width: 800,
        height: 600,
        scale: 0.0,
        title: s(""),
        resource: None,
        resource_user_data: std::ptr::null_mut(),
        log: None,
        log_user_data: std::ptr::null_mut(),
        log_level: 0,
        flags: ERK_APP_HEADLESS | ERK_APP_EMBEDDED_FONTS,
    };
    let before = LIVE.load(Ordering::SeqCst);
    let mut app = std::ptr::null_mut();
    assert_eq!(erk_app_create(&config, &mut app), ERK_OK);
    assert_eq!(erk_load_html(app, s(html)), ERK_OK);
    assert_eq!(erk_app_tick(app, 0), ERK_OK);
    let held = LIVE.load(Ordering::SeqCst) - before;
    assert_eq!(erk_app_destroy(app), ERK_OK);
    held
}

#[test]
fn a_laid_out_card_keeps_little() {
    let (few, many) = (500, 1500);
    let (small, large) = (cards_page(few), cards_page(many));
    // Once each first: what the process keeps after its first app (fonts
    // parsed, thread-locals) is not a card's.
    held(&small);
    let per_card = (held(&large) - held(&small)) / (many - few) as isize;
    eprintln!("a card holds {per_card} bytes");
    // Measured 2026-11-25: 19 477 bytes, and 27 685 with the room Parley
    // grew kept.
    assert!(per_card < 20 * 1024, "a card holds {per_card} bytes");
}
