#!/bin/sh
# Installs the latest system1router binary from GitHub releases.
#   curl -fsSL https://raw.githubusercontent.com/Valkea/System1Router/main/install.sh | sh
# SYSTEM1ROUTER_INSTALL_DIR overrides the target folder (default ~/.local/bin).
set -eu

repo=Valkea/System1Router
case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) target=x86_64-unknown-linux-gnu ;;
  Linux-aarch64 | Linux-arm64) target=aarch64-unknown-linux-gnu ;;
  Darwin-arm64) target=aarch64-apple-darwin ;;
  Darwin-x86_64) echo "Intel Macs are not supported: ONNX Runtime, which runs the local model, has no prebuilt library for them." >&2; exit 1 ;;
  *) echo "No prebuilt system1router for $(uname -s) $(uname -m); with Rust: cargo install system1router" >&2; exit 1 ;;
esac

dir=${SYSTEM1ROUTER_INSTALL_DIR:-$HOME/.local/bin}
base=${SYSTEM1ROUTER_RELEASE_URL:-https://github.com/$repo/releases/latest/download}
archive=system1router-$target.tar.gz
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

echo "Downloading $archive..."
curl -fsSL "$base/$archive" -o "$tmp/$archive"
curl -fsSL "$base/SHA256SUMS" -o "$tmp/SHA256SUMS"
expected=$(grep " $archive\$" "$tmp/SHA256SUMS" | cut -d' ' -f1)
if command -v sha256sum >/dev/null; then actual=$(sha256sum "$tmp/$archive" | cut -d' ' -f1); else actual=$(shasum -a 256 "$tmp/$archive" | cut -d' ' -f1); fi
[ -n "$expected" ] && [ "$expected" = "$actual" ] || { echo "Checksum mismatch for $archive" >&2; exit 1; }

tar -xzf "$tmp/$archive" -C "$tmp"
mkdir -p "$dir"
mv -f "$tmp/system1router" "$dir/system1router"
chmod 755 "$dir/system1router"
echo "Installed $("$dir/system1router" --version) in $dir"

case ":$PATH:" in
  *":$dir:"*) ;;
  *) echo "Add $dir to your PATH, e.g.: echo 'export PATH=\"$dir:\$PATH\"' >> ~/.$(basename "${SHELL:-sh}")rc" ;;
esac
echo
echo "Next: system1router install    (Claude Code mod, OpenCode plugin, local model)"
echo "      system1router toggle on"
