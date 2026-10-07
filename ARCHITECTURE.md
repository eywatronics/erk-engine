# Erk Engine Architecture

Erk is an embeddable HTML/CSS UI engine for desktop applications, written in
Rust, with no JavaScript engine: `<script>` is never run. The host application drives the document
through `NodeId`s and batched mutations; Erk styles, lays out and paints it and
reports user events back. It reuses mature Rust components and puts its original work
where none of them reach: inline layout, the embedding API and its C ABI,
incremental rendering and form controls.

This document has two parts: what the engine looks like **today**, and the
**target** architecture it grows into. The reasoning behind every decision,
including the alternatives that were rejected, is in
[docs/design/p1-embedded.md](docs/design/p1-embedded.md) and
[docs/design/p0-architecture.md](docs/design/p0-architecture.md) (Turkish). The
milestone plan is in [docs/plans/roadmap.md](docs/plans/roadmap.md).

## Today (M3: the library)

A single process with no networking. The host (the demo shell is the first)
uses the `erk` crate; the document lives on its UI thread, and only the
raster has a thread of its own (p1-contract §1.1).

```
UI thread (the host's)                       raster thread
┌──────────────────────────────────┐  plain  ┌───────────────────────────┐
│ host: erk-shell                  │  data   │ erk-renderer: raster      │
│ erk: App, window (winit), events │ ──────► │ vello_cpu / vello_hybrid  │
│ erk-renderer: engine             │ ◄────── │ font and image tables     │
│ (DOM, style, layout, display     │ frames  │                           │
│  list; frame on a helper thread  │         │                           │
│  with a large stack)             │         │                           │
└──────────────────────────────────┘         └───────────────────────────┘
```

The UI thread and the raster share no mutable state. The display list and
the font and image tables' updates are plain owned data: numbers, strings,
byte vectors. The
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
  opaque 64-bit value; a stale id is an error code, never a crash. The value
  is scrambled per application for namespace separation and stale-id
  isolation only: it is not a security token, and gives no authentication or
  protection against forged ids. All calls
  come from the UI thread; callbacks run on it, never during layout or paint.
  The contract is written in M0.5, before M1.
- **Engine core**: the arena DOM, Stylo, layout and paint. It does no file,
  network or process I/O and reads no clock or environment, which keeps it
  deterministic and makes content unable to reach the file system.

Erk runs no JavaScript, ever, and has no networking and no sandbox: the
content is the host's own and cannot execute code. Documents are parsed as by
a browser with scripting disabled, so `<noscript>` content shows. JavaScript
and TypeScript drive Erk from outside, as host languages (Node.js and Bun,
M6), like Python and Go. Every language binding goes through the C ABI
(`erk.h`), never the Rust API, so one boundary carries the thread, panic and
reentrancy checks for all of them; the document never holds a script object, so there
is no DOM/GC cycle to manage. CI checks that no JavaScript engine is in either
lock file (`check-no-js-engine.sh`). A separate renderer process remains possible because the messages are
plain data, but is not planned.

## Crates

| Crate | Responsibility | Arrives in |
|---|---|---|
| `erk-dom` | Arena DOM and the html5ever tree sink. Depends on no other `erk-*` crate | M0 |
| `erk-style` | Stylo adapter and style engine. A named `unsafe` exception, because Stylo's `TElement` requires five `unsafe fn`s | M0 |
| `erk-renderer` | The engine (document, input, style, layout, display list) on the caller's thread, the raster on its own | M0, split in M3.1 |
| `erk-shell` | The demo host, the first user of `erk`: files, resources, the counter. Depends on no project crate but `erk` | M0, on `erk` since M3.3 |
| `erk` | Idiomatic Rust embedding API: an app, its document on the UI thread, its raster, its window and event loop, the system's fonts | M3 |
| `erk-ffi` | The same API as a C ABI: `include/erk.h`, generated by cbindgen; a named `unsafe` exception, every exported function behind one guard | M3.5 |
| `erk-python` | Python package over the C ABI | M6 |

## Rules enforced in CI

- `unsafe` is forbidden workspace-wide, and every crate inherits that lint.
  The only exception is `erk-style`: Stylo's `TElement` declares five methods
  as `unsafe fn`, and implementing them violates the lint even with safe
  bodies. CI counts every `unsafe` keyword in the crate, which must be exactly
  those five `unsafe fn`s, and unsafe operations inside them are forbidden.
- `erk-dom` uses no reference counting (`Rc`, `Arc`), and nothing may switch
  that clippy ban off: a canary type checks that clippy still rejects it.
- `erk-dom` is a leaf; `erk-shell` depends on no project crate but `erk`,
  and `erk` on none but `erk-renderer`, on any platform or feature set.
- The renderer's public surface is its engine, its raster, plain data, and
  `render_html` for its own tests. What crosses to the raster (the display
  list, the font and image tables' updates) names no shared or borrowed
  type; `erk` and the shell never call `render_html`.
- html5ever and Stylo resolve to a single version of their atom crates.
- CI builds with `--locked`.
- Chrome reference expectations only go down with a written reason.
- Commit messages and pull requests name no AI tool.

Guards are added in the same pull request as the thing they protect, never
earlier and never later. The full schedule is in
[docs/design/p0-verification.md](docs/design/p0-verification.md).
