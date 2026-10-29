#!/bin/sh
# Builds the release artifacts from an existing `cargo build --release
# --workspace`: the binary tarball the tuxmix-bin AUR package installs,
# plus a .deb and an .rpm when cargo-deb / cargo-generate-rpm are there.
#
#   tools/release/package.sh [out-dir]     # default: target/dist
#
# The tarball holds the installed tree (usr/...), the same files the
# source PKGBUILD installs, so a package only copies it.
set -eu

cd "$(dirname "$0")/../.."
out=${1:-target/dist}
version=$(sed -n 's/^version = "\(.*\)"/\1/p' tuxmix-gui/Cargo.toml | head -1)
arch=$(uname -m)
name=tuxmix-$version-$arch

for bin in tuxmix-gui tuxmix-tui; do
    [ -x "target/release/$bin" ] || {
        echo "target/release/$bin is missing: run cargo build --release --workspace" >&2
        exit 1
    }
done

stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
root=$stage/$name
install -Dm755 target/release/tuxmix-gui "$root/usr/bin/tuxmix"
install -Dm755 target/release/tuxmix-tui "$root/usr/bin/tuxmix-tui"
install -Dm644 tuxmix-gui/assets/tuxmix.desktop "$root/usr/share/applications/tuxmix.desktop"
install -Dm644 tuxmix-gui/assets/tuxmix.svg "$root/usr/share/icons/hicolor/scalable/apps/tuxmix.svg"
install -Dm644 tuxmix-gui/assets/tuxmix-128.png "$root/usr/share/icons/hicolor/128x128/apps/tuxmix.png"
install -Dm644 tuxmix-gui/assets/tuxmix-256.png "$root/usr/share/icons/hicolor/256x256/apps/tuxmix.png"
install -Dm644 tools/udev/70-tuxmix.rules "$root/usr/lib/udev/rules.d/70-tuxmix.rules"
install -Dm644 README.md "$root/usr/share/doc/tuxmix/README.md"
install -Dm644 LICENSE "$root/usr/share/licenses/tuxmix/LICENSE"

mkdir -p "$out"
tar -C "$stage" --owner=0 --group=0 --numeric-owner -czf "$out/$name.tar.gz" "$name"
echo "$out/$name.tar.gz"

if command -v cargo-deb >/dev/null; then
    cargo deb -p tuxmix-gui --no-build --output "$out"
fi
if command -v cargo-generate-rpm >/dev/null; then
    cargo generate-rpm -p tuxmix-gui --output "$out"
fi
