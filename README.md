# Erk Engine

**Erk** is a desktop browser engine written in Rust.

- Built on mature Rust components: html5ever, Stylo, Taffy, Parley, Vello
- Original work where none of them reach: inline layout, the process model and
  sandbox, networking and security policy, and the browser shell
- Progress measured with the [Web Platform Tests](https://web-platform-tests.org/),
  not calendar dates

> Erk is at the very beginning. It cannot browse the web yet.

## Status

The current milestone is **M0 — first pixel**: open a local HTML file in a
single process and paint it in a window or to a PNG. Multi-process isolation
and sandboxing come in M3, JavaScript in M4. See the
[roadmap](docs/plans/roadmap.md) (Turkish) for every milestone and its
acceptance criterion.

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
  erk-shell/     window, event loop, messaging with the renderer
  erk-renderer/  layout, display list, paint
  erk-style/     CSS styling with Stylo
  erk-network/   network interface (from M2)
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
