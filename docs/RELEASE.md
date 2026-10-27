# Release and Packaging

A release ships the `system1router` binary for Linux and macOS (GitHub release, used by `install.sh`) and the crate (crates.io, used by `cargo install system1router` and `system1router update`). The local model is not part of either: it stays in `models/` (Git LFS) and the binary downloads it from GitHub on first use.

1. Bump `version` in `Cargo.toml`, run all checks (see `CLAUDE.md`), commit.
2. Check the crate holds the sources the binary embeds (OpenCode plugin, Claude Code mod) and nothing else:

   ```sh
   cargo package --list
   cargo publish --dry-run
   ```

3. Tag and push. `.github/workflows/release.yml` tests and builds `x86_64` and `aarch64` for Linux and macOS, then creates the release with `system1router-<target>.tar.gz` and `SHA256SUMS`:

   ```sh
   git tag v0.2.0 && git push origin v0.2.0
   ```

4. Publish the crate: `cargo publish` (needs `cargo login` once).

A new model gets a new name (`ettin-32m-effort-v2`): add its three files to `models/`, change the default `providers.local.model` in `src/config.rs`, and release. Installed copies of the old model are never overwritten in place.

Windows is not built: the shared model process uses Unix sockets.
