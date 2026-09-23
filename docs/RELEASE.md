# Release and Packaging

Build each supported target with a pinned Rust toolchain and archive the executable, `README.md`, and `docs/`:

```sh
cargo test --all-targets
cargo build --release --target x86_64-unknown-linux-gnu
cargo build --release --target aarch64-apple-darwin
cargo build --release --target x86_64-pc-windows-msvc
```

The one-shot protocol is the compatibility baseline. A daemon must not be enabled unless startup latency measurements justify it, and it must fall back to the one-shot executable without changing routing semantics. Run `system1router doctor`, installer idempotence tests, and host fixture tests before enabling routing in a release.
