# Contributing to Erk Engine

Thanks for your interest in Erk! This document describes how to get changes merged.

By participating you agree to follow our [Code of Conduct](CODE_OF_CONDUCT.md).

## Workflow

1. **Open an issue first.** Use the bug report or feature request template. For anything
   larger than a small fix, wait for a maintainer to agree on the approach before writing code.
2. **Create a branch** from `main` using the naming scheme below.
3. **Open a pull request** against `main`. Direct pushes to `main` are not allowed.
4. **CI must pass** and **at least one maintainer must approve** before merging.
5. PRs are merged with **squash** or **rebase** to keep a linear history.

## Branch names

| Prefix      | Use for                  | Example                    |
|-------------|--------------------------|----------------------------|
| `feat/`     | New features             | `feat/wgpu-renderer`       |
| `fix/`      | Bug fixes                | `fix/bidi-text-crash`      |
| `refactor/` | Code restructuring       | `refactor/dom-arena`       |
| `docs/`     | Documentation            | `docs/architecture-update` |
| `chore/`    | Tooling, deps, CI        | `chore/update-wgpu`        |
| `m0/`, `m1/`, ... | Milestone work     | `m0/first-pixel`           |

## Commit messages

We use [Conventional Commits](https://www.conventionalcommits.org/):

```
<type>(<scope>): <short summary>
```

- `feat(css): implement flexbox layout algorithm`
- `fix(ipc): resolve memory leak in message deserialization`
- `chore(deps): update wgpu to 0.19`

Common types: `feat`, `fix`, `refactor`, `perf`, `test`, `docs`, `chore`, `ci`.
The scope is usually a crate or subsystem (`dom`, `css`, `ipc`, `network`, `shell`, ...).

A clean, meaningful history matters: it makes `git bisect` usable on a codebase this large.
With a squash merge the PR title becomes the commit subject, so it follows the same format.

Commit messages and pull requests do not name AI tools and carry no co-author trailer other
than the maintainer's; CI checks this.

## Before you push

Run the same checks CI runs:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo build --workspace --locked
cargo test --workspace --locked
```

Clippy warnings are treated as errors. For a change that affects rendering, put the
Chrome reference scores in the PR description:

```sh
cargo test -p erk-renderer --test chrome_reference -- --nocapture
```

## License

Unless you explicitly state otherwise, any contribution intentionally submitted for
inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual
licensed under MIT OR Apache-2.0, without any additional terms or conditions.
