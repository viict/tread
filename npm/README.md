# tread

A terminal reader for markdown, CSV, JSON and source code — `less`, but it
understands what it is showing: collapsible headings, real tables, colored
links, and navigation across a corpus of linked documents.

```sh
npx @viict/tread README.md
```

Or install it so the binary is on your `PATH` as `tread`:

```sh
npm install -g @viict/tread
```

## What this package is

`tread` is a Rust program with no dependencies. This package is a small launcher
that downloads the build for your machine on first run, verifies its SHA-256
against the checksums published with the release, and then gets out of the way.
Nothing is downloaded at install time, so `npm ci --ignore-scripts` and pnpm's
default script blocking are not a problem.

The binary is kept per version under your platform's data directory:

| Platform | Location |
| --- | --- |
| Linux | `$XDG_DATA_HOME/tread/<version>/`, else `~/.local/share/tread/<version>/` |
| macOS | `~/Library/Application Support/tread/<version>/` |
| Windows | `%LOCALAPPDATA%\tread\<version>\` |

Older versions are removed once a new one has been fetched.

## Using a binary you already have

Set `TREAD_BINARY` and nothing is ever downloaded:

```sh
export TREAD_BINARY=/usr/local/bin/tread
```

That is the right answer for a machine with several user accounts sharing one
install, and for a CI runner with no route to the internet. If `TREAD_BINARY`
names something that is not there, the run fails — it never falls back to
downloading a second copy behind your back.

## Other ways to install

```sh
cargo install tread          # from crates.io, builds from source
```

Prebuilt archives for every supported platform, and the `SHA256SUMS` this
launcher verifies against, are on the
[releases page](https://github.com/viict/tread/releases).

## Licence

MIT OR Apache-2.0.
