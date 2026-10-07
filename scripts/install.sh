#!/bin/sh
# Install an official release binary without Rust or administrator privileges.
set -eu

usage() {
    cat <<'EOF'
Usage: install.sh [--version VERSION] [--prefix DIRECTORY]

Install rusty-tiles into DIRECTORY (default: $HOME/.local/bin).
VERSION may include the leading v; the default is the latest stable release.
Supports Linux (glibc) and macOS, on x86_64 and ARM64.
Windows: download the x86_64-pc-windows-msvc ZIP from GitHub Releases.
EOF
}

fail() { printf 'rusty-tiles: %s\n' "$*" >&2; exit 1; }
version=
prefix=${HOME:?HOME must be set}/.local/bin
while [ "$#" -gt 0 ]; do
    case "$1" in
        --version|--prefix)
            [ "$#" -ge 2 ] && [ -n "$2" ] || fail "$1 needs a value"
            case "$1" in --version) version=$2 ;; --prefix) prefix=$2 ;; esac
            shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) fail "unknown option: $1 (see --help)" ;;
    esac
done

for tool in curl tar mktemp install uname awk chmod; do
    command -v "$tool" >/dev/null 2>&1 || fail "required tool not found: $tool"
done
case "$(uname -m)" in
    x86_64|amd64) arch=x86_64 ;;
    aarch64|arm64) arch=aarch64 ;;
    *) fail "unsupported architecture: $(uname -m)" ;;
esac
case "$(uname -s)" in
    Linux)
        command -v getconf >/dev/null 2>&1 && getconf GNU_LIBC_VERSION >/dev/null 2>&1 \
            || fail "Linux binaries need glibc; on musl/Alpine, build with Cargo or use the container"
        target=$arch-unknown-linux-gnu ;;
    Darwin) target=$arch-apple-darwin ;;
    *) fail "unsupported operating system: $(uname -s); Windows binaries are on GitHub Releases" ;;
esac

if command -v sha256sum >/dev/null 2>&1; then
    checksum_tool=sha256sum
elif command -v shasum >/dev/null 2>&1; then
    checksum_tool=shasum
else
    fail "SHA-256 verification needs sha256sum or shasum"
fi

repo=https://github.com/BenDyson-Arch/rusty-tiles
download() { curl --fail --silent --show-error --location --retry 3 --proto '=https' --proto-redir '=https' "$1" -o "$2"; }
if [ -z "$version" ]; then
    # The redirect pins all subsequent downloads to the same stable release.
    release_url=$(curl --fail --silent --show-error --location --retry 3 \
        --proto '=https' --proto-redir '=https' --output /dev/null \
        --write-out '%{url_effective}' "$repo/releases/latest") \
        || fail "could not find the latest release; try --version VERSION"
    version=${release_url##*/}
fi
version=${version#v}
case "$version" in
    ''|*[!0-9A-Za-z.+-]*) fail "invalid version: $version" ;;
esac
case "$version" in
    [0-9]*.[0-9]*.[0-9]*) ;;
    *) fail "expected a release version such as 0.3.0" ;;
esac

tmp_dir=$(mktemp -d)
trap 'rm -rf "$tmp_dir"' EXIT
trap 'exit 1' HUP INT TERM
asset=rusty-tiles-$version-$target.tgz
base=$repo/releases/download/v$version
download "$base/$asset" "$tmp_dir/$asset" \
    || fail "no binary for v$version / $target; older releases are source-only (see README Build with Cargo)"
download "$base/SHA256SUMS" "$tmp_dir/SHA256SUMS" || fail "could not download release checksums"
expected=$(awk -v asset="$asset" '$2 == asset { print $1 }' "$tmp_dir/SHA256SUMS")
[ "${#expected}" -eq 64 ] || fail "missing or invalid checksum for $asset"
case "$expected" in *[!0-9a-fA-F]*) fail "invalid SHA-256 checksum" ;; esac
if [ "$checksum_tool" = sha256sum ]; then
    actual=$(sha256sum "$tmp_dir/$asset")
else
    actual=$(shasum -a 256 "$tmp_dir/$asset")
fi
actual=${actual%% *}
[ "$actual" = "$expected" ] || fail "checksum mismatch for $asset"
tar -xzf "$tmp_dir/$asset" -C "$tmp_dir" rusty-tiles
[ -f "$tmp_dir/rusty-tiles" ] && [ ! -L "$tmp_dir/rusty-tiles" ] || fail "release has no regular rusty-tiles binary"
chmod 755 "$tmp_dir/rusty-tiles"
"$tmp_dir/rusty-tiles" --version || fail "binary cannot run on this system; see the release's platform requirements"
mkdir -p "$prefix"
install -m 755 "$tmp_dir/rusty-tiles" "$prefix/rusty-tiles"
printf 'Installed %s/rusty-tiles\n' "$prefix"
case ":${PATH:-}:" in
    *":$prefix:"*) ;;
    *) printf 'Add %s to PATH to run rusty-tiles.\n' "$prefix" ;;
esac
