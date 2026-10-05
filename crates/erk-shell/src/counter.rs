//! The counter demo's host (M2.4): the first host logic, written against
//! the renderer's messages as a host will write it against the C-ABI (M3).
//!
//! When a page has a `#count` element and an `#increment` (and perhaps a
//! `#decrement`) button, the host keeps the number: a click whose path
//! passes through a button changes it, and the host sets `#count`'s text.
//! The renderer only shows it. Pages without them are left alone.

use erk_renderer::{EventKind, FromRenderer, ToRenderer};

/// The requests the host's queries go out under.
const COUNT: u64 = 1;
const INCREMENT: u64 = 2;
const DECREMENT: u64 = 3;

#[derive(Default)]
pub(crate) struct Counter {
    value: i64,
    /// The nodes, as the queries answer them.
    count: Option<Option<u64>>,
    increment: Option<Option<u64>>,
    decrement: Option<Option<u64>>,
    /// The next request a change goes out under.
    next: u64,
}

impl Counter {
    /// The host, and the questions it asks the page it was loaded with.
    pub(crate) fn start() -> (Self, Vec<ToRenderer>) {
        let ask = |request, id: &str| ToRenderer::Query {
            request,
            scope: None,
            selector: format!("#{id}"),
        };
        let host = Self {
            next: 100,
            ..Self::default()
        };
        let questions = vec![
            ask(COUNT, "count"),
            ask(INCREMENT, "increment"),
            ask(DECREMENT, "decrement"),
        ];
        (host, questions)
    }

    /// What the host does about `message`.
    pub(crate) fn on(&mut self, message: &FromRenderer) -> Vec<ToRenderer> {
        match message {
            FromRenderer::QueryResult { request, result } => {
                let node = result.ok().flatten();
                match *request {
                    COUNT => self.count = Some(node),
                    INCREMENT => self.increment = Some(node),
                    DECREMENT => self.decrement = Some(node),
                    _ => {}
                }
                Vec::new()
            }
            FromRenderer::Event(event) if event.kind == EventKind::Click => {
                let (Some(Some(count)), Some(Some(increment))) = (self.count, self.increment)
                else {
                    return Vec::new();
                };
                let on = |button: u64| event.path.contains(&button);
                let step = if on(increment) {
                    1
                } else if self.decrement.flatten().is_some_and(on) {
                    -1
                } else {
                    return Vec::new();
                };
                self.value += step;
                self.next += 1;
                vec![ToRenderer::SetText {
                    request: self.next,
                    node: count,
                    text: self.value.to_string(),
                }]
            }
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::mpsc::Receiver;
    use std::time::Duration;

    use erk_renderer::{Frame, Modifiers, PointerButton, PointerInput, PointerKind};

    use super::*;

    const PAGE: &str = include_str!("../../../examples/counter.html");
    const PATIENCE: Duration = Duration::from_secs(60);

    fn decode(png_bytes: &[u8]) -> (u32, u32, Vec<u8>) {
        let decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
        let mut reader = decoder.read_info().expect("valid PNG");
        let mut pixels = vec![0; reader.output_buffer_size().expect("sized PNG")];
        let info = reader.next_frame(&mut pixels).expect("PNG frame");
        pixels.truncate(info.buffer_size());
        (info.width, info.height, pixels)
    }

    /// Run the host on what the renderer says until `done` is true of a
    /// message; the last frame seen.
    fn pump(
        host: &mut Counter,
        to: &std::sync::mpsc::Sender<ToRenderer>,
        from: &Receiver<FromRenderer>,
        mut done: impl FnMut(&FromRenderer) -> bool,
    ) -> Option<Frame> {
        let mut frame = None;
        loop {
            let message = from.recv_timeout(PATIENCE).expect("the renderer answers");
            for answer in host.on(&message) {
                to.send(answer).unwrap();
            }
            let stop = done(&message);
            if let FromRenderer::Frame(painted) = message {
                frame = Some(painted);
            }
            if stop {
                return frame;
            }
        }
    }

    /// The acceptance item: a click goes to the renderer, the host counts,
    /// and the frame showing the new number is the golden image.
    #[test]
    fn clicks_count_and_the_page_shows_the_number() {
        // The buttons' boxes, to click their middles.
        let boxes = erk_renderer::element_boxes(PAGE, 800, 600, &mut |_| None);
        let buttons: Vec<_> = boxes.iter().filter(|b| b.tag == "button").collect();
        let middle = |b: &erk_renderer::ElementBox| (b.x + b.width / 2.0, b.y + b.height / 2.0);
        let (decrement, increment) = (middle(buttons[0]), middle(buttons[1]));

        let (to, from, renderer) = erk_renderer::spawn();
        to.send(ToRenderer::Load {
            html: PAGE.to_owned(),
        })
        .unwrap();
        to.send(ToRenderer::Resize {
            width: 800,
            height: 600,
        })
        .unwrap();
        let (mut host, questions) = Counter::start();
        for question in questions {
            to.send(question).unwrap();
        }
        // The answers, and the first frame: input is hit-tested against the
        // last frame painted, and before one there is nothing to click. (The
        // answers come during the batch the frame is painted after.)
        let (mut answered, mut painted) = (false, false);
        pump(&mut host, &to, &from, |m| {
            answered |= matches!(
                m,
                FromRenderer::QueryResult {
                    request: DECREMENT,
                    ..
                }
            );
            painted |= matches!(m, FromRenderer::Frame(_));
            answered && painted
        });
        assert!(host.count.flatten().is_some() && host.increment.flatten().is_some());

        let click = |(x, y): (f32, f32)| {
            for kind in [PointerKind::Down, PointerKind::Up] {
                to.send(ToRenderer::Pointer(PointerInput {
                    kind,
                    x,
                    y,
                    button: PointerButton::Primary,
                    modifiers: Modifiers::default(),
                }))
                .unwrap();
            }
        };
        // Up three times, down once: 2. Each change is answered, then painted.
        let mut frame = None;
        for target in [increment, increment, increment, decrement] {
            click(target);
            let changed = host.next + 1;
            let mut answered = false;
            frame = pump(&mut host, &to, &from, |m| {
                if let FromRenderer::Done { request, result } = m
                    && *request == changed
                {
                    assert_eq!(*result, Ok(()));
                    answered = true;
                }
                answered && matches!(m, FromRenderer::Frame(_))
            });
        }
        assert_eq!(host.value, 2);
        let frame = frame.expect("a frame after the last change");
        assert!(
            frame.display_list().contains("\"2\""),
            "{}",
            frame.display_list()
        );

        // Leave the pointer's state out of the image: move it away.
        to.send(ToRenderer::Pointer(PointerInput {
            kind: PointerKind::Leave,
            x: 0.0,
            y: 0.0,
            button: PointerButton::None,
            modifiers: Modifiers::default(),
        }))
        .unwrap();
        let frame = pump(&mut host, &to, &from, |m| {
            matches!(m, FromRenderer::Frame(_))
        })
        .unwrap();

        let actual = frame.to_png().unwrap();
        let golden = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/counter-2.png");
        if std::env::var("ERK_BLESS").is_ok_and(|value| value == "1") {
            std::fs::create_dir_all(golden.parent().unwrap()).unwrap();
            std::fs::write(&golden, &actual).unwrap();
        } else {
            let expected = std::fs::read(&golden).unwrap_or_else(|_| {
                panic!(
                    "no golden image at {}; run with ERK_BLESS=1",
                    golden.display()
                )
            });
            if decode(&expected) != decode(&actual) {
                let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/golden-actual");
                std::fs::create_dir_all(&out).unwrap();
                std::fs::write(out.join("counter-2.png"), &actual).unwrap();
                panic!("the counter differs from {}", golden.display());
            }
        }
        to.send(ToRenderer::Shutdown).unwrap();
        renderer.join().unwrap();
    }

    #[test]
    fn a_click_anywhere_inside_a_button_counts() {
        let (mut host, _) = Counter::start();
        for (request, node) in [(COUNT, 1), (INCREMENT, 2), (DECREMENT, 3)] {
            host.on(&FromRenderer::QueryResult {
                request,
                result: Ok(Some(node)),
            });
        }
        let click = |target, path: Vec<u64>| {
            FromRenderer::Event(erk_renderer::Event {
                kind: EventKind::Click,
                target,
                path,
                x: 0.0,
                y: 0.0,
                modifiers: Modifiers::default(),
            })
        };
        // An icon (9) inside the increment button, then the decrement
        // button twice, then elsewhere.
        let texts: Vec<String> = [
            click(9, vec![9, 2, 5]),
            click(3, vec![3, 5]),
            click(3, vec![3, 5]),
            click(5, vec![5]),
        ]
        .iter()
        .flat_map(|event| host.on(event))
        .map(|change| match change {
            ToRenderer::SetText { node: 1, text, .. } => text,
            other => panic!("{other:?}"),
        })
        .collect();
        assert_eq!(texts, ["1", "0", "-1"]);
    }

    #[test]
    fn a_page_without_the_counter_is_left_alone() {
        let (mut host, _) = Counter::start();
        for request in [COUNT, INCREMENT, DECREMENT] {
            host.on(&FromRenderer::QueryResult {
                request,
                result: Ok(None),
            });
        }
        let click = FromRenderer::Event(erk_renderer::Event {
            kind: EventKind::Click,
            target: 7,
            path: vec![7, 8],
            x: 0.0,
            y: 0.0,
            modifiers: Modifiers::default(),
        });
        assert!(host.on(&click).is_empty());
    }
}
