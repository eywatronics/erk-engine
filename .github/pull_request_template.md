## Summary

<!-- What does this PR change and why? -->

Closes #

## Checklist

- [ ] PR title follows Conventional Commits (e.g. `feat(css): implement flexbox layout`)
- [ ] `cargo fmt --all` has been run
- [ ] `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` passes
- [ ] `cargo test --workspace --locked` passes
- [ ] Tests added or updated where it makes sense
- [ ] Rendering changes: the Chrome reference score table
      (`cargo test -p erk-renderer --test chrome_reference -- --nocapture`) is below
