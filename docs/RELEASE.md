# Release and Packaging

Build each supported target with a pinned Rust toolchain and archive the executable, `README.md`, and `docs/`:

```sh
cargo test --all-targets
cargo build --release --target x86_64-unknown-linux-gnu
cargo build --release --target aarch64-apple-darwin
cargo build --release --target x86_64-pc-windows-msvc
```

The executable embeds the OpenCode server/TUI plugin sources so `system1router install` can install them at either scope. Check that Cargo packages those assets, then publish each version to crates.io to enable `system1router update`:

```sh
cargo package --list
cargo publish --dry-run
cargo publish
```

The one-shot protocol is the compatibility baseline. A daemon must not be enabled unless startup latency measurements justify it, and it must fall back to the one-shot executable without changing routing semantics. Run `system1router doctor`, installer idempotence tests, and host fixture tests before enabling routing in a release.
