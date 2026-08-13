"use strict";

// Pulling one file out of a release archive, with node's zlib and nothing else.
//
// The release publishes a `.tar.gz` per unix target and a `.zip` per Windows
// one, each holding the binary beside the README, the licences and `docs/`.
// Those archives are what `install.sh` verifies and what a person downloads by
// hand, so the npm launcher reads the same bytes rather than asking the release
// to carry a second, npm-shaped copy of every binary.
//
// node has `zlib` built in and no archive reader. Both formats are simple
// enough to walk directly: a tar is 512-byte headers each followed by its
// file's bytes, and a zip is a central directory of entries pointing at
// deflate streams `zlib.inflateRawSync` already understands. That is about a
// hundred lines here against a dependency in a package that has none — and
// against three megabytes of duplicated binary on every release.
//
// Both readers are handed an archive whose SHA-256 has *already* been checked
// against the release's `SHA256SUMS` (see `bin/tread.js`), so this code parses
// bytes that are known to be the ones the release published. It is deliberately
// strict anyway: unknown shapes return null rather than guessing, and nothing
// here writes to disk or follows a path out of the archive — the caller asks
// for one base name and gets bytes back.

const zlib = require("zlib");

/// A NUL-terminated fixed-width field, as tar writes them.
function cstr(buf) {
  const end = buf.indexOf(0);
  return buf.toString("utf8", 0, end === -1 ? buf.length : end);
}

/// The last path component, for either separator.
function base(name) {
  const cut = Math.max(name.lastIndexOf("/"), name.lastIndexOf("\\"));
  return cut === -1 ? name : name.slice(cut + 1);
}

/// The bytes of the file named `want` inside a gzipped tar, or null.
///
/// Only regular files are considered: type `0`, or a NUL for the pre-POSIX
/// spelling of the same thing. Every other entry — directories, long-name
/// extensions, anything unrecognised — is skipped rather than interpreted.
function fromTarGz(packed, want) {
  const tar = zlib.gunzipSync(packed);
  let at = 0;
  while (at + 512 <= tar.length) {
    const head = tar.subarray(at, at + 512);
    if (head[0] === 0) {
      return null; // the two zero blocks that end an archive
    }
    const name = cstr(head.subarray(0, 100));
    const size = parseInt(cstr(head.subarray(124, 136)).trim() || "0", 8);
    if (!Number.isFinite(size) || size < 0 || at + 512 + size > tar.length) {
      return null;
    }
    const type = head[156];
    const body = at + 512;
    if ((type === 0x30 || type === 0) && base(name) === want) {
      return tar.subarray(body, body + size);
    }
    at = body + Math.ceil(size / 512) * 512;
  }
  return null;
}

/// Offset of the end-of-central-directory record, searching from the end.
///
/// The record is 22 bytes plus a comment of up to 64 KiB, so the search window
/// is bounded by the format rather than by the file.
function endOfDirectory(buf) {
  const floor = Math.max(0, buf.length - (22 + 0xffff));
  for (let at = buf.length - 22; at >= floor; at--) {
    if (buf.readUInt32LE(at) === 0x06054b50) {
      return at;
    }
  }
  return -1;
}

/// The bytes of the file named `want` inside a zip, or null.
///
/// Stored (method 0) and deflated (method 8) entries are read; anything else is
/// skipped. Sizes come from the central directory, which is authoritative even
/// when the local header defers them to a data descriptor.
function fromZip(buf, want) {
  const eocd = endOfDirectory(buf);
  if (eocd === -1) {
    return null;
  }
  const count = buf.readUInt16LE(eocd + 10);
  let at = buf.readUInt32LE(eocd + 16);
  for (let i = 0; i < count; i++) {
    if (at + 46 > buf.length || buf.readUInt32LE(at) !== 0x02014b50) {
      return null;
    }
    const method = buf.readUInt16LE(at + 10);
    const packed = buf.readUInt32LE(at + 20);
    const nameLen = buf.readUInt16LE(at + 28);
    const extraLen = buf.readUInt16LE(at + 30);
    const commentLen = buf.readUInt16LE(at + 32);
    const local = buf.readUInt32LE(at + 42);
    const name = buf.toString("utf8", at + 46, at + 46 + nameLen);
    if (base(name) === want && (method === 0 || method === 8)) {
      return inflateAt(buf, local, packed, method);
    }
    at += 46 + nameLen + extraLen + commentLen;
  }
  return null;
}

/// Read one entry's data, given where its *local* header sits. The local
/// header repeats the name and extra fields at its own lengths, which are the
/// only way to know where the data starts.
function inflateAt(buf, local, packed, method) {
  if (local + 30 > buf.length || buf.readUInt32LE(local) !== 0x04034b50) {
    return null;
  }
  const nameLen = buf.readUInt16LE(local + 26);
  const extraLen = buf.readUInt16LE(local + 28);
  const from = local + 30 + nameLen + extraLen;
  if (from + packed > buf.length) {
    return null;
  }
  const raw = buf.subarray(from, from + packed);
  return method === 0 ? raw : zlib.inflateRawSync(raw);
}

/// Pull `want` out of whichever archive shape `asset` names.
function extract(asset, bytes, want) {
  return asset.endsWith(".zip")
    ? fromZip(bytes, want)
    : fromTarGz(bytes, want);
}

module.exports = { extract, fromTarGz, fromZip };
