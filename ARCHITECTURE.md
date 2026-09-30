# Erk Engine Architecture

Erk is an embeddable HTML/CSS UI engine for desktop applications, written in
Rust, with no JavaScript. The host application drives the document through
`NodeId`s and batched mutations; Erk styles, lays out and paints it and reports
user events back. It reuses mature Rust components and puts its original work
where none of them reach: inline layout, the embedding API and its C ABI,
incremental rendering and form controls.

This document has two parts: what the engine looks like **today**, and the
**target** architecture it grows into. The reasoning behind every decision,
including the alternatives that were rejected, is in
[docs/design/p1-embedded.md](docs/design/p1-embedded.md) and
[docs/design/p0-architecture.md](docs/design/p0-architecture.md) (Turkish). The
milestone plan is in [docs/plans/roadmap.md](docs/plans/roadmap.md).

## Today (M0: first pixel)

A single process with no networking. The engine reads a local HTML file and
paints it.

```
main thread                         renderer thread
┌────────────────────┐   messages   ┌──────────────────────────────────┐
│ erk-shell          │ ───────────► │ erk-renderer                     │
│ window (winit),    │ ◄─────────── │ DOM, style, layout, paint        │
│ input, frames      │   (mpsc)     │                                  │
└────────────────────┘              └──────────────────────────────────┘
```

The shell and the renderer share no mutable state. They talk only through
typed messages that hold plain owned data: text, integers, pixel bytes. The
same discipline becomes the embedding API and its C ABI in M3, and keeps a
separate renderer process possible later without a rewrite.

## Rendering pipeline

```
HTML ─► html5ever ─► erk-dom ─► Stylo ─► Taffy + Parley ─► display list ─► vello_cpu ─► window / PNG
```

| Stage | Component | Notes |
|---|---|---|
| Parsing | `html5ever` (pinned to 0.39.0) | Upgraded together with Stylo; both must share one atom crate version |
| DOM | `erk-dom` | Arena of nodes addressed by `NodeId` (u32 index + u32 generation); no reference counting |
| Style | Stylo, via `erk-style` | Servo's and Firefox's CSS engine; sequential traversal for now |
| Layout | Taffy; Erk's inline layout from M1 | Taffy handles block, flexbox, grid and floats. In M0 a paragraph is one Taffy leaf shaped by Parley; inline formatting (line boxes, spans across lines, justification) is Erk's own work in M1 |
| Text | Parley | HarfRust shaping, ICU4X segmentation, fontique font fallback |
| Paint | Erk display list → `vello_cpu` | CPU rendering is the default and the reference for tests; a GPU path (`vello_hybrid` on wgpu) comes in M2 |

## Target architecture

```
host application (Rust, C, Python)     logic, state, files, network
        |   ^
        |   |  NodeIds, batched mutations, events
        v   |
erk (Rust API)  --  erk-ffi (C ABI, erk.h)
        |
engine core: erk-dom, erk-style, erk-renderer     no I/O, no clock, no env
        |
window: winit + softbuffer (vello_hybrid on the GPU from M2)
```

- **Host application**: owns all logic and all I/O. It gives Erk a resource
  callback (for CSS `url()` and images), the current time, and configuration.
- **erk / erk-ffi**: one API, as idiomatic Rust and as a C ABI. `NodeId` is an
  opaque 64-bit value; a stale id is an error code, never a crash. All calls
  come from the UI thread; callbacks run on it, never during layout or paint.
  The contract is written in M0.5, before M1.
- **Engine core**: the arena DOM, Stylo, layout and paint. It does no file,
  network or process I/O and reads no clock or environment, which keeps it
  deterministic and makes content unable to reach the file system.

There is no JavaScript, no networking and no sandbox: the content is the host's
own. A separate renderer process remains possible because the messages are
plain data, but is not planned.

## Crates

| Crate | Responsibility | Arrives in |
|---|---|---|
| `erk-dom` | Arena DOM and the html5ever tree sink. Depends on no other `erk-*` crate | M0 |
| `erk-style` | Stylo adapter and style engine. A named `unsafe` exception, because Stylo's `TElement` requires five `unsafe fn`s | M0 |
| `erk-renderer` | Layout, display list, paint; the renderer thread and its messages | M0 |
| `erk-shell` | Window, event loop, the demo host. Does not depend on `erk-dom` or `erk-style` | M0 |
| `erk` | Idiomatic Rust embedding API | M3 |
| `erk-ffi` | The same API as a C ABI (`erk.h`); a named `unsafe` exception | M3 |
| `erk-python` | Python package over the C ABI | M6 |

## Rules enforced in CI

- `unsafe` is forbidden workspace-wide, and every crate inherits that lint.
  The only exception is `erk-style`: Stylo's `TElement` declares five methods
  as `unsafe fn`, and implementing them violates the lint even with safe
  bodies. CI counts every `unsafe` keyword in the crate, which must be exactly
  those five `unsafe fn`s, and unsafe operations inside them are forbidden.
- `erk-dom` uses no reference counting (`Rc`, `Arc`), and nothing may switch
  that clippy ban off: a canary type checks that clippy still rejects it.
- `erk-dom` is a leaf; `erk-shell` depends on neither `erk-dom` nor
  `erk-style` directly, on any platform or feature set.
- The renderer's public surface is its thread, plain-data messages, `to_png`
  and `render_html` for its own tests; the shell never calls `render_html`.
- html5ever and Stylo resolve to a single version of their atom crates.
- CI builds with `--locked`.
- Chrome reference expectations only go down with a written reason.
- Commit messages and pull requests name no AI tool.

Guards are added in the same pull request as the thing they protect, never
earlier and never later. The full schedule is in
[docs/design/p0-verification.md](docs/design/p0-verification.md).
