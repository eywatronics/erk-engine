# Erk Engine

**Erk** is an embeddable HTML/CSS UI engine for desktop applications, written
in Rust. The engine core runs no JavaScript: your application (in Rust, and
later C, Python, Go or, optionally, JavaScript) owns the logic and drives the
document, Erk lays it out, paints it and reports what the user did.

- Built on mature Rust components: html5ever, Stylo, Taffy, Parley, Vello
- Original work where none of them reach: inline layout, the embedding API
  and its C ABI, incremental rendering and form controls
- The engine core does no file, network or process I/O and reads no clock or
  environment: resources and time come from the host
- Progress measured by tests, not calendar dates: a pixel comparison with
  Chrome today, the CSS [Web Platform Tests](https://web-platform-tests.org/)
  from M1

> Erk is at the very beginning: it renders a static page, and cannot yet be
> embedded or interacted with.

## Status

**M0 — first pixel** is done: Erk paints a local HTML file in a window or to a
PNG. Next is **M0.5**, the embedding contract (C ABI, threading, memory and
callback rules) written before more code depends on it, then **M1 — static
UI**: inline layout, flexbox, borders and images. The Rust API and C ABI come
in M3, a mutable DOM with events in M4, form controls in M5 and Python in M6.
See the [roadmap](docs/plans/roadmap.md) (Turkish) for every milestone and its
acceptance criterion, and [the design](docs/design/p1-embedded.md) (Turkish)
for why Erk is no longer a browser.

## Building

Requirements:

- A recent stable [Rust toolchain](https://rustup.rs/); the exact version is
  pinned in `rust-toolchain.toml`
- On Windows: the MSVC Build Tools with the C++ workload
- Python 3 (used by Stylo's build script)

```sh
cargo build
cargo test
```

Open a local HTML file in a window, or paint it to a PNG:

```sh
cargo run -p erk-shell -- examples/merhaba.html
cargo run -p erk-shell -- --screenshot out.png examples/merhaba.html
```

## Project layout

```
crates/
  erk-shell/     window, event loop, the demo host
  erk-renderer/  layout, display list, paint; the renderer thread
  erk-style/     CSS styling with Stylo
  erk-dom/       arena DOM and HTML parsing
docs/
  design/        architecture decisions (Turkish)
  plans/         roadmap and milestone plans (Turkish)
```

See [ARCHITECTURE.md](ARCHITECTURE.md) for the design overview.

## Contributing

Contributions are welcome! Please read [CONTRIBUTING.md](CONTRIBUTING.md) and our
[Code of Conduct](CODE_OF_CONDUCT.md) first.

## License

Erk Engine is dual-licensed under either:

* MIT License ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)
* Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)

at your option.

The embedded Noto Sans font files in `crates/erk-renderer/assets/fonts` are
licensed under the [SIL Open Font License 1.1](crates/erk-renderer/assets/fonts/OFL.txt).

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
