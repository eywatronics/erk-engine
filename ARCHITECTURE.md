# Erk Engine Architecture

Erk is a desktop browser engine written in Rust. It reuses mature Rust
components and puts its original work where none of them reach: inline layout,
the process model and sandbox, the networking and security policy, and the
browser shell.

This document has two parts: what the engine looks like **today**, and the
**target** architecture it grows into. The reasoning behind every decision,
including the alternatives that were rejected, is in
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
typed messages that own their data. When the renderer moves into its own
process (M3), the transport changes and nothing else does.

## Rendering pipeline

```
HTML ─► html5ever ─► erk-dom ─► Stylo ─► Taffy + Parley ─► display list ─► vello_cpu ─► window / PNG
```

| Stage | Component | Notes |
|---|---|---|
| Parsing | `html5ever` (pinned to 0.39.0) | Upgraded together with Stylo; both must share one atom crate version |
| DOM | `erk-dom` | Arena of nodes addressed by `NodeId` (u32 index + u32 generation); no reference counting |
| Style | Stylo, via `erk-style` | Servo's and Firefox's CSS engine; sequential traversal for now |
| Layout | Taffy + Erk's inline layout | Taffy handles block, flexbox, grid and floats. Inline formatting (line boxes, spans across lines, justification) is Erk's own work |
| Text | Parley | HarfRust shaping, ICU4X segmentation, fontique font fallback |
| Paint | Erk display list → `vello_cpu` | CPU rendering is the default and the reference for tests; a GPU path (`vello_hybrid` on wgpu) comes in M2 |

## Target architecture

```
┌──────────────────────────────┐
│  Shell / broker (erk-shell)  │  privileged: window, input, process lifecycle
└──────┬───────────────┬───────┘
       │ IPC           │ IPC
┌──────▼───────┐ ┌─────▼────────┐
│  Renderer    │ │  Network     │  sockets, DNS, TLS, cookies, cache
│ (sandboxed,  │ │  process     │
│  per site)   │ │ erk-network  │
└──────────────┘ └──────────────┘
```

- **Shell / broker**: the only privileged process. Owns the window, routes
  input and supervises the other processes.
- **Renderer**: one sandboxed process per site (scheme + registrable domain).
  It has no file system or network access.
- **Network process**: the sole owner of sockets, TLS, cookies and the HTTP
  cache. It implements the Fetch standard and enforces CORS, CORP and ORB
  before any bytes reach a renderer. It never trusts an origin claimed by a
  renderer.

A crash in a renderer must never take down the shell.

## Crates

| Crate | Responsibility | Arrives in |
|---|---|---|
| `erk-dom` | Arena DOM and the html5ever tree sink. Depends on no other `erk-*` crate | M0 |
| `erk-style` | Stylo adapter and style engine. The one crate allowed to declare `unsafe fn`s, because Stylo's `TElement` requires five | M0 |
| `erk-renderer` | Layout, display list, paint | M0 |
| `erk-shell` | Window, event loop, messaging with the renderer. Does not depend on `erk-dom` | M0 |
| `erk-network` | Network interface: a temporary HTTP client in M2, Erk's own Fetch implementation in M6 | M2 |
| `erk-ipc` | Cross-process transport | M3 |
| `erk-sandbox` | OS sandbox APIs; the only crate allowed `unsafe` | M3 |
| `erk-js` | JavaScript engine bindings | M4 |

## JavaScript

JavaScript arrives in M4, after static pages render and can be browsed. The
engine is chosen by measurement on the same mini DOM. The candidates in order
are Boa (pure Rust) and SpiderMonkey (via `mozjs`). V8 is not a candidate. The
acceptance test is independent of the engine: a detached DOM subtree held only
by its own event listener's closure must be collected.

## Rules enforced in CI

- `unsafe` is forbidden workspace-wide. The only exception is `erk-style`:
  Stylo's `TElement` declares five methods as `unsafe fn`, and implementing
  them violates the lint even with safe bodies. CI checks that exactly those
  five signatures are allowed and that no unsafe block exists.
- `erk-dom` uses no `std::rc::Rc`.
- `erk-dom` is a leaf; `erk-shell` does not depend on `erk-dom` directly.
- html5ever and Stylo resolve to a single version of their atom crates.

Guards are added in the same pull request as the thing they protect, never
earlier and never later. The full schedule is in
[docs/design/p0-verification.md](docs/design/p0-verification.md).
