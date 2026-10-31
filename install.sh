#!/bin/sh
# Installs (or updates) Jiary from the latest GitHub release:
#
#   curl -fsSL https://raw.githubusercontent.com/barcanjocs/jiary/main/install.sh | sh
#
# Picks the build for this OS and CPU, checks it against its published
# SHA-256, and puts the `jiary` binary in ~/.local/bin, no sudo needed.
# Environment:
#   JIARY_INSTALL_DIR   where to put the binary (default: ~/.local/bin)
#   JIARY_DOWNLOAD_URL  where the archives are (default: the latest release)
set -eu

repo="barcanjocs/jiary"
base="${JIARY_DOWNLOAD_URL:-https://github.com/$repo/releases/latest/download}"
dir="${JIARY_INSTALL_DIR:-$HOME/.local/bin}"

fail() {
    echo "jiary: $*" >&2
    exit 1
}

case "$(uname -s)" in
    Darwin) os=apple-darwin ;;
    Linux) os=unknown-linux-gnu ;;
    *) fail "no prebuilt binary for $(uname -s); build from source instead (see the README)" ;;
esac
case "$(uname -m)" in
    x86_64 | amd64) arch=x86_64 ;;
    arm64 | aarch64) arch=aarch64 ;;
    *) fail "no prebuilt binary for $(uname -m); build from source instead (see the README)" ;;
esac
name="jiary-$arch-$os"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "Downloading $name..."
curl -fsSL "$base/$name.tar.gz" -o "$tmp/$name.tar.gz" ||
    fail "download failed: $base/$name.tar.gz"
curl -fsSL "$base/$name.tar.gz.sha256" -o "$tmp/$name.tar.gz.sha256" ||
    fail "download failed: $base/$name.tar.gz.sha256"

# The checksum file names the archive, so check it from its directory.
if command -v sha256sum >/dev/null 2>&1; then
    (cd "$tmp" && sha256sum -c "$name.tar.gz.sha256" >/dev/null 2>&1)
else
    (cd "$tmp" && shasum -a 256 -c "$name.tar.gz.sha256" >/dev/null 2>&1)
fi || fail "checksum mismatch for $name.tar.gz; not installing"

tar xzf "$tmp/$name.tar.gz" -C "$tmp"
mkdir -p "$dir"
cp "$tmp/$name/jiary" "$dir/jiary.new"
chmod 755 "$dir/jiary.new"
# Replaced in one step, so a running jiary keeps its old binary.
mv "$dir/jiary.new" "$dir/jiary"

echo "Installed jiary to $dir/jiary"
case ":$PATH:" in
    *":$dir:"*) echo "Run it with: jiary" ;;
    *)
        echo "$dir isn't on your PATH yet. Add it, e.g.:"
        echo "  echo 'export PATH=\"$dir:\$PATH\"' >> ~/.profile"
        ;;
esac
