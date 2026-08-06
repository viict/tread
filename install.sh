#!/bin/sh
#
# Install tread.
#
#   curl -fsSL https://raw.githubusercontent.com/viict/tread/master/install.sh | sh
#
# Environment:
#   INSTALL_PATH   where to put the binary (default: $HOME/.local/bin)
#   VERSION        which release to install (default: the latest)
#
# Everything is inside main(), called on the last line. A pipe into sh can be
# cut off mid-transfer, and a script that runs as it parses would then execute
# half of itself; this way an incomplete download does nothing at all.
set -eu

REPO="viict/tread"
BIN="tread"

say() { printf '%s\n' "$*"; }
die() { printf 'install: %s\n' "$*" >&2; exit 1; }

need() {
    command -v "$1" > /dev/null 2>&1 || die "this needs $1, which is not installed"
}

# Whichever downloader is present. macOS ships curl, most Linux images ship one
# or the other, and being wrong about it is the most likely way this fails.
fetch() {
    if command -v curl > /dev/null 2>&1; then
        curl -fsSL "$1" -o "$2"
    elif command -v wget > /dev/null 2>&1; then
        wget -qO "$2" "$1"
    else
        die "this needs curl or wget, and has neither"
    fi
}

# Linux has sha256sum, macOS has shasum. Verifying is the point of publishing
# checksums, so a missing tool is a refusal rather than a silent skip.
checksum() {
    if command -v sha256sum > /dev/null 2>&1; then
        sha256sum "$1" | cut -d' ' -f1
    elif command -v shasum > /dev/null 2>&1; then
        shasum -a 256 "$1" | cut -d' ' -f1
    else
        die "this needs sha256sum or shasum to verify the download"
    fi
}

# uname -> the Rust target triple the release is named after.
target_triple() {
    os=$(uname -s)
    arch=$(uname -m)
    case "$os" in
        Linux) suffix="unknown-linux-musl" ;;
        Darwin) suffix="apple-darwin" ;;
        MINGW* | MSYS* | CYGWIN*)
            die "Windows is not installed by this script; take the .zip from https://github.com/$REPO/releases"
            ;;
        *) die "unsupported system: $os" ;;
    esac
    case "$arch" in
        x86_64 | amd64) cpu="x86_64" ;;
        aarch64 | arm64) cpu="aarch64" ;;
        *) die "unsupported architecture: $arch" ;;
    esac
    printf '%s-%s' "$cpu" "$suffix"
}

# The newest release tag, read from the API without needing jq.
latest_version() {
    tmp="$1/release.json"
    fetch "https://api.github.com/repos/$REPO/releases/latest" "$tmp" ||
        die "could not reach the GitHub API to find the latest release"
    v=$(sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$tmp" | head -1)
    [ -n "$v" ] || die "could not find a release; set VERSION=vX.Y.Z to choose one"
    printf '%s' "$v"
}

main() {
    need tar
    dest=${INSTALL_PATH:-$HOME/.local/bin}

    tmp=$(mktemp -d 2>/dev/null || mktemp -d -t tread)
    # Leave nothing behind, on success or on any failure.
    trap 'rm -rf "$tmp"' EXIT INT TERM

    triple=$(target_triple)
    version=${VERSION:-$(latest_version "$tmp")}
    name="$BIN-$version-$triple"
    base="https://github.com/$REPO/releases/download/$version"

    say "tread $version for $triple"

    fetch "$base/$name.tar.gz" "$tmp/$name.tar.gz" ||
        die "no build for $triple in $version — see https://github.com/$REPO/releases"

    # Verify against the release's own SHA256SUMS. A checksum that is missing
    # or does not match stops the install; a binary is not worth guessing about.
    if fetch "$base/SHA256SUMS" "$tmp/SHA256SUMS" 2>/dev/null; then
        want=$(grep " $name.tar.gz\$" "$tmp/SHA256SUMS" | cut -d' ' -f1 || true)
        [ -n "$want" ] || die "$name.tar.gz is not listed in SHA256SUMS"
        got=$(checksum "$tmp/$name.tar.gz")
        [ "$want" = "$got" ] || die "checksum mismatch for $name.tar.gz — refusing to install"
        say "checksum ok"
    else
        die "could not download SHA256SUMS — refusing to install unverified"
    fi

    tar -xzf "$tmp/$name.tar.gz" -C "$tmp"
    [ -f "$tmp/$name/$BIN" ] || die "the archive does not contain $BIN"

    mkdir -p "$dest" || die "could not create $dest"
    # Install to a temporary name in the same directory and move it into place,
    # so a running tread is replaced atomically rather than truncated.
    cp "$tmp/$name/$BIN" "$dest/.$BIN.new"
    chmod 755 "$dest/.$BIN.new"
    mv -f "$dest/.$BIN.new" "$dest/$BIN" || die "could not write to $dest"

    say "installed $dest/$BIN"

    case ":$PATH:" in
        *":$dest:"*) ;;
        *)
            say ""
            say "$dest is not on your PATH. Add it with:"
            say ""
            say "    export PATH=\"\$PATH:$dest\""
            ;;
    esac

    "$dest/$BIN" --version
}

main "$@"
