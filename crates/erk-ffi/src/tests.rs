//! The C ABI against p1-contract §11, called as C calls it.

use std::sync::atomic::{AtomicU32, Ordering};

use super::*;

fn s(text: &str) -> ErkStr {
    lent(text)
}

fn config(flags: u32) -> ErkConfig {
    ErkConfig {
        struct_size: size_of::<ErkConfig>() as u32,
        width: 200,
        height: 100,
        scale: 0.0,
        title: s(""),
        resource: None,
        resource_user_data: std::ptr::null_mut(),
        log: None,
        log_user_data: std::ptr::null_mut(),
        log_level: 0,
        flags,
    }
}

const HEADLESS: u32 = ERK_APP_HEADLESS | ERK_APP_EMBEDDED_FONTS;

fn app_with(html: &str) -> *mut ErkApp {
    let mut app = std::ptr::null_mut();
    assert_eq!(erk_app_create(&config(HEADLESS), &mut app), ERK_OK);
    assert_eq!(erk_load_html(app, s(html)), ERK_OK);
    assert_eq!(erk_app_tick(app, 0), ERK_OK);
    app
}

fn query(app: *mut ErkApp, selector: &str) -> ErkNodeId {
    let mut found = ERK_NODE_NONE;
    assert_eq!(
        erk_query(app, ERK_NODE_NONE, s(selector), &mut found),
        ERK_OK
    );
    assert_ne!(found, ERK_NODE_NONE, "{selector}");
    found
}

fn text_of(app: *mut ErkApp, node: ErkNodeId) -> Result<String, ErkStatus> {
    let mut buf = [0u8; 256];
    let mut len = 0;
    match erk_node_text(app, node, buf.as_mut_ptr().cast(), buf.len(), &mut len) {
        ERK_OK => Ok(String::from_utf8(buf[..len].to_vec()).unwrap()),
        status => Err(status),
    }
}

const PAGE: &str = r#"<body style="margin: 0"><button id="b" style="display: block; width: 100px; height: 40px">düğme</button><p id="p">metin</p>"#;

#[test]
fn the_abi_version_is_0_4() {
    assert_eq!(erk_abi_version(), 4);
}

fn change(kind: u32, node: ErkNodeId, parent: ErkNodeId, name: &str, value: &str) -> ErkMutation {
    ErkMutation {
        struct_size: size_of::<ErkMutation>() as u32,
        kind,
        node,
        parent,
        before: ERK_NODE_NONE,
        name: s(name),
        value: s(value),
    }
}

#[test]
fn a_batch_is_applied_from_c_and_names_what_it_made() {
    let app = app_with(PAGE);
    let body = query(app, "body");
    let new = |at: u64| ERK_NEW_NODE + at;
    let mut batch = [
        change(ERK_MUTATION_CREATE_ELEMENT, 0, 0, "", "ul"),
        change(ERK_MUTATION_CREATE_ELEMENT, 0, 0, "", "li"),
        change(ERK_MUTATION_SET_TEXT, new(1), 0, "", "iki"),
        change(ERK_MUTATION_APPEND, new(1), new(0), "", ""),
        change(ERK_MUTATION_CREATE_ELEMENT, 0, 0, "", "li"),
        change(ERK_MUTATION_CREATE_TEXT, 0, 0, "", "bir"),
        change(ERK_MUTATION_APPEND, new(5), new(4), "", ""),
        change(ERK_MUTATION_INSERT_BEFORE, new(4), new(0), "", ""),
        change(ERK_MUTATION_SET_ATTR, new(0), 0, "id", "liste"),
        change(ERK_MUTATION_ADD_CLASS, new(4), 0, "", "ilk"),
        change(ERK_MUTATION_APPEND, new(0), body, "", ""),
    ];
    batch[7].before = new(1);
    let mut created = [7u64; 11];
    let mut failed_at = 99;
    assert_eq!(
        erk_apply(
            app,
            batch.as_ptr(),
            batch.len(),
            created.as_mut_ptr(),
            &mut failed_at
        ),
        ERK_OK
    );
    assert_eq!(failed_at, 99, "untouched on success");
    let list = query(app, "#liste");
    assert_eq!(created[0], list);
    assert_eq!(query(app, "li.ilk"), created[4]);
    assert_eq!(created[3], ERK_NODE_NONE);
    assert_eq!(text_of(app, list).unwrap(), "biriki");

    // query_all: all of them, or the count when they do not fit.
    let mut found = [ERK_NODE_NONE; 2];
    let mut len = 0;
    assert_eq!(
        erk_query_all(
            app,
            list,
            s("li"),
            found.as_mut_ptr(),
            found.len(),
            &mut len
        ),
        ERK_OK
    );
    assert_eq!((len, found), (2, [created[4], created[1]]));
    let mut one = [ERK_NODE_NONE; 1];
    assert_eq!(
        erk_query_all(app, ERK_NODE_NONE, s("li"), one.as_mut_ptr(), 1, &mut len),
        ERK_ERR_BUFFER_TOO_SMALL
    );
    assert_eq!((len, one), (2, [ERK_NODE_NONE]));
    assert_eq!(
        erk_query_all(
            app,
            ERK_NODE_NONE,
            s(".yok"),
            std::ptr::null_mut(),
            0,
            &mut len
        ),
        ERK_OK
    );
    assert_eq!(len, 0);

    // The first failure stops it and says where; the ones before stay.
    let failing = [
        change(ERK_MUTATION_CREATE_ELEMENT, 0, 0, "", "p"),
        change(ERK_MUTATION_APPEND, new(0), body, "", ""),
        // The body into its own paragraph.
        change(ERK_MUTATION_APPEND, body, new(0), "", ""),
        change(ERK_MUTATION_CREATE_ELEMENT, 0, 0, "", "hr"),
    ];
    assert_eq!(
        erk_apply(
            app,
            failing.as_ptr(),
            failing.len(),
            std::ptr::null_mut(),
            &mut failed_at
        ),
        ERK_ERR_INVALID_ARGUMENT
    );
    assert_eq!(failed_at, 2);
    assert_eq!(
        erk_query_all(
            app,
            ERK_NODE_NONE,
            s("hr"),
            std::ptr::null_mut(),
            0,
            &mut len
        ),
        ERK_OK
    );
    assert_eq!(len, 0, "nothing after the failure ran");
    // A kind it does not know is refused before anything runs.
    let unknown = [
        change(ERK_MUTATION_CREATE_ELEMENT, 0, 0, "", "hr"),
        change(99, 0, 0, "", ""),
    ];
    assert_eq!(
        erk_apply(
            app,
            unknown.as_ptr(),
            2,
            std::ptr::null_mut(),
            &mut failed_at
        ),
        ERK_ERR_INVALID_ARGUMENT
    );
    assert_eq!(failed_at, 1);
    assert_eq!(
        erk_query_all(
            app,
            ERK_NODE_NONE,
            s("hr"),
            std::ptr::null_mut(),
            0,
            &mut len
        ),
        ERK_OK
    );
    assert_eq!(len, 0);
    // A stale node is stale in a batch too.
    let stale = [change(ERK_MUTATION_REMOVE, list, 0, "", "")];
    assert_eq!(
        erk_apply(
            app,
            stale.as_ptr(),
            1,
            std::ptr::null_mut(),
            std::ptr::null_mut()
        ),
        ERK_OK
    );
    assert_eq!(
        erk_apply(app, stale.as_ptr(), 1, std::ptr::null_mut(), &mut failed_at),
        ERK_ERR_STALE_NODE
    );
    assert_eq!(failed_at, 0);
    assert_eq!(erk_app_destroy(app), ERK_OK);
}

/// The key and text of the last key event a subscription saw.
#[derive(Default)]
struct LastKey {
    key: AtomicU32,
    kind: AtomicU32,
    typed: std::sync::Mutex<String>,
}

#[allow(unsafe_code)] // SAFETY: a C callback; dereferences the test's LastKey and Erk's event.
unsafe extern "C" fn remember_key(user_data: *mut c_void, _: *mut ErkApp, event: *const ErkEvent) {
    // SAFETY: the test passes a live LastKey and Erk a live event whose
    // text it lends for the call.
    let (last, event) = unsafe { (&*user_data.cast::<LastKey>(), &*event) };
    // SAFETY: as above.
    let typed = unsafe { std::slice::from_raw_parts(event.text.ptr.cast::<u8>(), event.text.len) };
    last.key.store(event.key, Ordering::SeqCst);
    last.kind.store(event.kind, Ordering::SeqCst);
    *last.typed.lock().unwrap() = String::from_utf8(typed.to_vec()).unwrap();
}

#[test]
fn a_key_event_names_its_key_and_what_it_typed() {
    let app = app_with(PAGE);
    let last = LastKey::default();
    let mut subscription = 0;
    let user_data = std::ptr::from_ref(&last).cast_mut().cast();
    for kind in [ERK_EVENT_KEY_DOWN, ERK_EVENT_KEY_UP] {
        assert_eq!(
            erk_on(
                app,
                query(app, "body"),
                kind,
                Some(remember_key),
                user_data,
                None,
                &mut subscription
            ),
            ERK_OK
        );
    }
    let key = |kind, key, text| ErkInput {
        struct_size: size_of::<ErkInput>() as u32,
        kind,
        x: 0.0,
        y: 0.0,
        dx: 0.0,
        dy: 0.0,
        button: 0,
        key,
        text: s(text),
        modifiers: 0,
    };
    let seen = || {
        (
            last.kind.load(Ordering::SeqCst),
            last.key.load(Ordering::SeqCst),
            last.typed.lock().unwrap().clone(),
        )
    };
    assert_eq!(
        erk_app_input(app, &key(ERK_INPUT_KEY_DOWN, ERK_KEY_CHARACTER, "ğ")),
        ERK_OK
    );
    assert_eq!(
        seen(),
        (ERK_EVENT_KEY_DOWN, ERK_KEY_CHARACTER, "ğ".to_owned())
    );
    assert_eq!(
        erk_app_input(app, &key(ERK_INPUT_KEY_UP, ERK_KEY_ESCAPE, "")),
        ERK_OK
    );
    assert_eq!(seen(), (ERK_EVENT_KEY_UP, ERK_KEY_ESCAPE, String::new()));
    assert_eq!(erk_app_destroy(app), ERK_OK);
}

#[test]
fn a_document_is_built_from_c() {
    let app = app_with(PAGE);
    let body = query(app, "body");
    let (mut list, mut item, mut text) = (ERK_NODE_NONE, ERK_NODE_NONE, ERK_NODE_NONE);
    assert_eq!(erk_node_create(app, s("UL"), &mut list), ERK_OK);
    assert_eq!(erk_node_create(app, s("li"), &mut item), ERK_OK);
    assert_eq!(erk_text_create(app, s("bir"), &mut text), ERK_OK);
    assert_eq!(erk_node_append(app, item, text), ERK_OK);
    assert_eq!(
        erk_node_insert_before(app, list, item, ERK_NODE_NONE),
        ERK_OK
    );
    assert_eq!(erk_node_append(app, body, list), ERK_OK);
    assert_eq!(query(app, "body > ul > li"), item);
    assert_eq!(text_of(app, list).unwrap(), "bir");
    // Attributes and classes.
    assert_eq!(erk_node_set_attr(app, item, s("data-n"), s("1")), ERK_OK);
    assert_eq!(erk_node_add_class(app, item, s("yeni")), ERK_OK);
    let mut has = 0;
    assert_eq!(erk_node_has_class(app, item, s("yeni"), &mut has), ERK_OK);
    assert_eq!(has, 1);
    assert_eq!(erk_node_remove_class(app, item, s("yeni")), ERK_OK);
    assert_eq!(erk_node_has_class(app, item, s("yeni"), &mut has), ERK_OK);
    assert_eq!(has, 0);
    let mut buf = [0u8; 8];
    let mut len = 0;
    assert_eq!(
        erk_node_attr(
            app,
            item,
            s("data-n"),
            buf.as_mut_ptr().cast(),
            buf.len(),
            &mut len
        ),
        ERK_OK
    );
    assert_eq!(&buf[..len], b"1");
    assert_eq!(erk_node_remove_attr(app, item, s("data-n")), ERK_OK);
    assert_eq!(
        erk_node_remove_attr(app, item, s("data-n")),
        ERK_ERR_NOT_FOUND
    );
    assert_eq!(
        erk_node_attr(
            app,
            item,
            s("data-n"),
            buf.as_mut_ptr().cast(),
            buf.len(),
            &mut len
        ),
        ERK_ERR_NOT_FOUND
    );
    // What DOM refuses, and bad names.
    assert_eq!(erk_node_append(app, item, list), ERK_ERR_INVALID_ARGUMENT);
    let mut bad = ERK_NODE_NONE;
    assert_eq!(
        erk_node_create(app, s("a b"), &mut bad),
        ERK_ERR_INVALID_ARGUMENT
    );
    // Removed: stale everywhere.
    assert_eq!(erk_node_remove(app, list), ERK_OK);
    assert_eq!(erk_node_append(app, body, item), ERK_ERR_STALE_NODE);
    assert_eq!(text_of(app, text), Err(ERK_ERR_STALE_NODE));
    let mut root = ERK_NODE_NONE;
    assert_eq!(erk_document_root(app, &mut root), ERK_OK);
    assert_eq!(erk_node_remove(app, root), ERK_ERR_INVALID_ARGUMENT);
    assert_eq!(erk_app_destroy(app), ERK_OK);
}

#[test]
fn removing_a_subscribed_node_runs_its_destroy_once() {
    let app = app_with(PAGE);
    let counts = Counts::default();
    let mut subscription = 0;
    let user_data = std::ptr::from_ref(&counts).cast_mut().cast();
    let button = query(app, "#b");
    assert_eq!(
        erk_on(
            app,
            button,
            ERK_EVENT_CLICK,
            Some(count_click),
            user_data,
            Some(count_destroy),
            &mut subscription
        ),
        ERK_OK
    );
    assert_eq!(erk_node_remove(app, button), ERK_OK);
    assert_eq!(counts.destroyed.load(Ordering::SeqCst), 1);
    assert_eq!(erk_off(app, subscription), ERK_ERR_NOT_FOUND);
    assert_eq!(erk_app_destroy(app), ERK_OK);
    assert_eq!(counts.destroyed.load(Ordering::SeqCst), 1);
}

/// An app pointer, carried to another thread for a call there.
struct Carried(*mut ErkApp);
// SAFETY: the test calls from the other thread only what must refuse it,
// or the thread-safe functions.
#[allow(unsafe_code)]
unsafe impl Send for Carried {}

#[test]
fn a_call_from_another_thread_does_nothing() {
    let app = app_with(PAGE);
    let p = query(app, "#p");
    let carried = Carried(app);
    let statuses = std::thread::spawn(move || {
        let app = carried;
        let mut out = ERK_NODE_NONE;
        [
            erk_load_html(app.0, s("<p>başka</p>")),
            erk_node_set_text(app.0, p, s("değişti")),
            erk_query(app.0, ERK_NODE_NONE, s("p"), &mut out),
            erk_app_tick(app.0, 1),
            erk_app_destroy(app.0),
        ]
    })
    .join()
    .unwrap();
    assert_eq!(statuses, [ERK_ERR_WRONG_THREAD; 5]);
    assert_eq!(text_of(app, p).unwrap(), "metin");
    assert_eq!(erk_app_destroy(app), ERK_OK);
}

static POSTED_RAN: AtomicU32 = AtomicU32::new(0);
static POSTED_DESTROYED: AtomicU32 = AtomicU32::new(0);

#[allow(unsafe_code)] // SAFETY: a C callback; reads only what the test passed.
unsafe extern "C" fn posted(user_data: *mut c_void, app: *mut ErkApp) {
    let node = user_data as ErkNodeId;
    assert_eq!(erk_node_set_text(app, node, s("arka plandan")), ERK_OK);
    // The loop may not run inside a callback.
    assert_eq!(erk_app_tick(app, 9), ERK_ERR_REENTRANT);
    assert_eq!(erk_app_destroy(app), ERK_ERR_REENTRANT);
    POSTED_RAN.fetch_add(1, Ordering::SeqCst);
}

#[allow(unsafe_code)] // SAFETY: a C callback; touches no pointer.
unsafe extern "C" fn posted_destroy(_: *mut c_void) {
    POSTED_DESTROYED.fetch_add(1, Ordering::SeqCst);
}

#[test]
fn work_posted_from_another_thread_runs_on_the_ui_thread_and_is_destroyed_once() {
    let app = app_with(PAGE);
    let p = query(app, "#p");
    let carried = Carried(app);
    let status = std::thread::spawn(move || {
        let app = carried;
        erk_app_post(app.0, Some(posted), p as *mut c_void, Some(posted_destroy))
    })
    .join()
    .unwrap();
    assert_eq!(status, ERK_OK);
    assert_eq!(POSTED_RAN.load(Ordering::SeqCst), 0, "not before the tick");
    assert_eq!(erk_app_tick(app, 1), ERK_OK);
    assert_eq!(POSTED_RAN.load(Ordering::SeqCst), 1);
    assert_eq!(POSTED_DESTROYED.load(Ordering::SeqCst), 1);
    assert_eq!(text_of(app, p).unwrap(), "arka plandan");
    assert_eq!(erk_app_destroy(app), ERK_OK);
    assert_eq!(POSTED_DESTROYED.load(Ordering::SeqCst), 1, "once");
}

#[test]
fn another_apps_ids_are_stale_and_zero_is_no_node() {
    let (one, two) = (app_with(PAGE), app_with(PAGE));
    let p = query(one, "#p");
    assert_eq!(text_of(two, p), Err(ERK_ERR_STALE_NODE));
    assert_eq!(text_of(one, ERK_NODE_NONE), Err(ERK_ERR_INVALID_ARGUMENT));
    assert_eq!(erk_app_destroy(one), ERK_OK);
    // A destroyed app's ids are stale in a new one.
    let three = app_with(PAGE);
    assert_eq!(text_of(three, p), Err(ERK_ERR_STALE_NODE));
    for app in [two, three] {
        assert_eq!(erk_app_destroy(app), ERK_OK);
    }
}

#[test]
fn a_panic_poisons_the_app_and_only_destroy_works() {
    let app = app_with(PAGE);
    // What any exported function does when the engine panics under it.
    assert_eq!(
        guard(app, Calls::Document, |_| panic!("injected")),
        ERK_ERR_PANIC
    );
    let mut out = ERK_NODE_NONE;
    assert_eq!(
        erk_query(app, ERK_NODE_NONE, s("p"), &mut out),
        ERK_ERR_POISONED
    );
    assert_eq!(erk_app_tick(app, 1), ERK_ERR_POISONED);
    assert_eq!(erk_app_destroy(app), ERK_OK);
}

#[test]
fn a_null_app_and_bad_text_are_invalid_arguments() {
    assert_eq!(
        erk_load_html(std::ptr::null_mut(), s("")),
        ERK_ERR_INVALID_ARGUMENT
    );
    let app = app_with(PAGE);
    let invalid = [0xff_u8, 0xfe];
    let bad = ErkStr {
        ptr: invalid.as_ptr().cast(),
        len: invalid.len(),
    };
    assert_eq!(erk_load_html(app, bad), ERK_ERR_INVALID_ARGUMENT);
    let dangling = ErkStr {
        ptr: std::ptr::null(),
        len: 3,
    };
    assert_eq!(erk_load_html(app, dangling), ERK_ERR_INVALID_ARGUMENT);
    // The document is the one before.
    assert_eq!(text_of(app, query(app, "#p")).unwrap(), "metin");
    assert_eq!(erk_app_destroy(app), ERK_OK);
}

#[test]
fn a_short_structure_is_read_and_written_as_far_as_it_goes() {
    // A config from an older header: only struct_size, width and height.
    let mut short = config(0);
    short.struct_size = offset_of!(ErkConfig, scale) as u32;
    // Fields past the size are not read: a headless flag there is ignored,
    // so this is a windowed app, never run.
    short.flags = HEADLESS;
    let mut app = std::ptr::null_mut();
    assert_eq!(erk_app_create(&short, &mut app), ERK_OK);
    assert_eq!(erk_app_tick(app, 0), ERK_OK);
    let mut frame = ErkFrame {
        struct_size: size_of::<ErkFrame>() as u32,
        width: 0,
        height: 0,
        rgba: std::ptr::null(),
        len: 0,
    };
    assert_eq!(
        erk_app_frame(app, &mut frame),
        ERK_ERR_NOT_FOUND,
        "no window, no frame"
    );
    assert_eq!(erk_app_destroy(app), ERK_OK);
    // Too short for the size itself.
    short.struct_size = 2;
    assert_eq!(erk_app_create(&short, &mut app), ERK_ERR_INVALID_ARGUMENT);

    // A box from an older header: up to the border box only.
    let app = app_with(PAGE);
    let mut found = ErkBox {
        struct_size: offset_of!(ErkBox, margin) as u32,
        x: -1.0,
        y: -1.0,
        width: -1.0,
        height: -1.0,
        margin: [-7.0; 4],
        border: [-7.0; 4],
        padding: [-7.0; 4],
    };
    assert_eq!(erk_node_box(app, query(app, "#b"), &mut found), ERK_OK);
    assert_eq!(
        (found.x, found.y, found.width, found.height),
        (0.0, 0.0, 100.0, 40.0)
    );
    assert_eq!(found.margin, [-7.0; 4], "past the size: not written");
    assert_eq!(found.struct_size, offset_of!(ErkBox, margin) as u32);
    assert_eq!(erk_app_destroy(app), ERK_OK);
}

#[test]
fn a_buffer_too_small_is_not_written() {
    let app = app_with(PAGE);
    let p = query(app, "#p");
    let mut buf = [b'-'; 3];
    let mut len = 0;
    assert_eq!(
        erk_node_text(app, p, buf.as_mut_ptr().cast(), buf.len(), &mut len),
        ERK_ERR_BUFFER_TOO_SMALL
    );
    assert_eq!((len, buf), ("metin".len(), [b'-'; 3]));
    assert_eq!(erk_app_destroy(app), ERK_OK);
}

/// Counts a subscription's clicks and its destroy.
#[derive(Default)]
struct Counts {
    clicks: AtomicU32,
    destroyed: AtomicU32,
}

#[allow(unsafe_code)] // SAFETY: a C callback; dereferences the test's Counts and Erk's event.
unsafe extern "C" fn count_click(user_data: *mut c_void, _: *mut ErkApp, event: *const ErkEvent) {
    // SAFETY: the test passes a live Counts and Erk a live event.
    let (counts, event) = unsafe { (&*user_data.cast::<Counts>(), &*event) };
    assert_eq!(event.kind, ERK_EVENT_CLICK);
    counts.clicks.fetch_add(1, Ordering::SeqCst);
}

#[allow(unsafe_code)] // SAFETY: a C callback; dereferences the test's Counts.
unsafe extern "C" fn count_destroy(user_data: *mut c_void) {
    // SAFETY: the test passes a live Counts.
    let counts = unsafe { &*user_data.cast::<Counts>() };
    counts.destroyed.fetch_add(1, Ordering::SeqCst);
}

fn subscribe_counts(app: *mut ErkApp, node: ErkNodeId, counts: &Counts) -> ErkStatus {
    let mut subscription = 0;
    let user_data = std::ptr::from_ref(counts).cast_mut().cast();
    let status = erk_on(
        app,
        node,
        ERK_EVENT_CLICK,
        Some(count_click),
        user_data,
        Some(count_destroy),
        &mut subscription,
    );
    if status == ERK_OK {
        assert_eq!(erk_off(app, subscription), ERK_OK);
        assert_eq!(erk_off(app, subscription), ERK_ERR_NOT_FOUND);
    }
    status
}

#[test]
fn destroy_runs_once_however_a_subscription_ends() {
    // By erk_off (in subscribe_counts).
    let app = app_with(PAGE);
    let by_off = Counts::default();
    assert_eq!(subscribe_counts(app, query(app, "#b"), &by_off), ERK_OK);
    assert_eq!(by_off.destroyed.load(Ordering::SeqCst), 1);

    // By the node leaving the document, and by the app's end.
    let (removed, ended) = (Counts::default(), Counts::default());
    let mut subscription = 0;
    for (node, counts) in [(query(app, "#p"), &removed), (query(app, "#b"), &ended)] {
        let user_data = std::ptr::from_ref(counts).cast_mut().cast();
        assert_eq!(
            erk_on(
                app,
                node,
                ERK_EVENT_CLICK,
                Some(count_click),
                user_data,
                Some(count_destroy),
                &mut subscription
            ),
            ERK_OK
        );
    }
    let body = query(app, "body");
    // Clicking still counts once per click on the button.
    let click = |kind| ErkInput {
        struct_size: size_of::<ErkInput>() as u32,
        kind,
        x: 50.0,
        y: 20.0,
        dx: 0.0,
        dy: 0.0,
        button: ERK_BUTTON_PRIMARY,
        key: 0,
        text: s(""),
        modifiers: 0,
    };
    assert_eq!(erk_app_input(app, &click(ERK_INPUT_POINTER_DOWN)), ERK_OK);
    assert_eq!(erk_app_input(app, &click(ERK_INPUT_POINTER_UP)), ERK_OK);
    assert_eq!(ended.clicks.load(Ordering::SeqCst), 1);
    // The body's text replaces its children: the paragraph leaves.
    assert_eq!(erk_node_set_text(app, body, s("boş")), ERK_OK);
    assert_eq!(removed.destroyed.load(Ordering::SeqCst), 1);
    assert_eq!(
        ended.destroyed.load(Ordering::SeqCst),
        1,
        "the button left with it"
    );

    // A subscription that is not made takes nothing.
    let refused = Counts::default();
    assert_eq!(subscribe_counts(app, query(app, "body"), &refused), ERK_OK);
    let stale = Counts::default();
    let gone = query(app, "body");
    assert_eq!(erk_load_html(app, s(PAGE)), ERK_OK);
    assert_eq!(subscribe_counts(app, gone, &stale), ERK_ERR_STALE_NODE);
    assert_eq!(stale.destroyed.load(Ordering::SeqCst), 0);
    let unknown = Counts::default();
    let mut out = 0;
    let user_data = std::ptr::from_ref(&unknown).cast_mut().cast();
    assert_eq!(
        erk_on(
            app,
            query(app, "#b"),
            ERK_EVENT_SUBMIT,
            Some(count_click),
            user_data,
            Some(count_destroy),
            &mut out
        ),
        ERK_ERR_INVALID_ARGUMENT
    );
    assert_eq!(unknown.destroyed.load(Ordering::SeqCst), 0);

    // With the app.
    let with_app = Counts::default();
    let user_data = std::ptr::from_ref(&with_app).cast_mut().cast();
    assert_eq!(
        erk_on(
            app,
            query(app, "#b"),
            ERK_EVENT_CLICK,
            Some(count_click),
            user_data,
            Some(count_destroy),
            &mut out
        ),
        ERK_OK
    );
    assert_eq!(erk_app_destroy(app), ERK_OK);
    assert_eq!(with_app.destroyed.load(Ordering::SeqCst), 1);
}

static RESOURCE_ASKED: AtomicU32 = AtomicU32::new(0);

#[allow(unsafe_code)] // SAFETY: a C callback; reads the URL Erk lends it.
unsafe extern "C" fn ask(_: *mut c_void, app: *mut ErkApp, request: u64, kind: u32, url: ErkStr) {
    assert_eq!(kind, ERK_RESOURCE_IMAGE);
    assert_eq!(text(url).unwrap(), "a.png");
    // The document may be read inside the callback.
    let mut root = ERK_NODE_NONE;
    assert_eq!(erk_document_root(app, &mut root), ERK_OK);
    RESOURCE_ASKED.store(u32::try_from(request).unwrap(), Ordering::SeqCst);
}

/// A 1 × 1 opaque red PNG.
const RED: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53,
    0xde, 0x00, 0x00, 0x00, 0x0c, 0x49, 0x44, 0x41, 0x54, 0x08, 0xd7, 0x63, 0xf8, 0xcf, 0xc0, 0x00,
    0x00, 0x03, 0x01, 0x01, 0x00, 0x18, 0xdd, 0x8d, 0xb0, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e,
    0x44, 0xae, 0x42, 0x60, 0x82,
];

#[test]
fn a_resource_is_answered_from_another_thread() {
    let mut with_provider = config(HEADLESS);
    with_provider.resource = Some(ask);
    let mut app = std::ptr::null_mut();
    assert_eq!(erk_app_create(&with_provider, &mut app), ERK_OK);
    let page = r#"<body style="margin: 0; background: #123456"><img src="a.png" style="display: block; width: 20px; height: 20px">"#;
    assert_eq!(erk_load_html(app, s(page)), ERK_OK);
    assert_eq!(erk_app_tick(app, 0), ERK_OK);
    let request = u64::from(RESOURCE_ASKED.load(Ordering::SeqCst));
    assert_ne!(request, 0);
    let carried = Carried(app);
    let status = std::thread::spawn(move || {
        let app = carried;
        erk_resource_complete(
            app.0,
            request,
            ERK_OK,
            s("image/png"),
            RED.as_ptr(),
            RED.len(),
        )
    })
    .join()
    .unwrap();
    assert_eq!(status, ERK_OK);
    // Answered once: a second answer finds no request.
    assert_eq!(
        erk_resource_complete(app, request, ERK_OK, s(""), RED.as_ptr(), RED.len()),
        ERK_ERR_NOT_FOUND
    );
    assert_eq!(erk_app_tick(app, 1), ERK_OK);
    let mut frame = ErkFrame {
        struct_size: size_of::<ErkFrame>() as u32,
        width: 0,
        height: 0,
        rgba: std::ptr::null(),
        len: 0,
    };
    assert_eq!(erk_app_frame(app, &mut frame), ERK_OK);
    #[allow(unsafe_code)] // SAFETY: the frame's pixels are valid until the next tick.
    let pixels = unsafe { std::slice::from_raw_parts(frame.rgba, frame.len) };
    let at = (10 * frame.width as usize + 10) * 4;
    assert_eq!(&pixels[at..at + 3], &[255, 0, 0]);
    assert_eq!(erk_app_destroy(app), ERK_OK);
}
