#!/usr/bin/env node
"use strict";

// The npm entry point for tread.
//
// This file is a launcher, not the program. `tread` is a Rust binary with no
// dependencies; npm carries a five-kilobyte shim that finds the right build for
// this machine, fetches it once, and hands the terminal over to it.
//
// Three rules shape everything below:
//
//   * Nothing runs that was not verified. The archive's SHA-256 is checked
//     against the SHA256SUMS published with the release before the file is put
//     anywhere it could be executed. A download that does not match is deleted
//     and the run fails loudly.
//   * The binary is state, not cache. It lives under the platform's *data*
//     directory, because a cache is something the OS may delete — macOS purges
//     ~/Library/Caches under disk pressure — and a binary that vanishes leaves
//     an offline machine unable to run tread at all. The table below mirrors
//     src/plat/dirs.rs; if one changes, change both.
//   * A version is a directory. Upgrading needs no invalidation logic: a new
//     version writes a new directory and the old ones are pruned once the new
//     binary has run.
//
// TREAD_BINARY points at a tread you already have and skips all of it, which is
// what a single global install on a multi-user machine wants, and what a locked
// down CI runner needs. It is honoured exactly: if it names something that is
// not there, the run fails rather than quietly downloading a second copy.
//
// Zero dependencies here too — only node's own https, crypto and fs, plus the
// archive reader in ../lib/unpack.js. The launcher reads the very archives the
// releases page publishes and `install.sh` verifies; the release is not asked
// to carry a second, npm-shaped copy of every binary.

const { spawnSync } = require("child_process");
const crypto = require("crypto");
const fs = require("fs");
const https = require("https");
const os = require("os");
const path = require("path");

const { extract } = require("../lib/unpack.js");

const VERSION = require("../package.json").version;
const REPO = "viict/tread";
const MAX_BYTES = 32 * 1024 * 1024;
const MAX_REDIRECTS = 5;

/// node's platform-arch pair to the Rust target its release asset is named for.
const TARGETS = {
  "linux-x64": "x86_64-unknown-linux-musl",
  "linux-arm64": "aarch64-unknown-linux-musl",
  "darwin-x64": "x86_64-apple-darwin",
  "darwin-arm64": "aarch64-apple-darwin",
  "win32-x64": "x86_64-pc-windows-msvc",
  "win32-arm64": "aarch64-pc-windows-msvc",
};

function die(message) {
  console.error(`tread: ${message}`);
  process.exit(1);
}

/// Where downloaded binaries live, before the version is appended.
///
/// An explicitly set XDG_DATA_HOME wins everywhere, including macOS: a user who
/// exports it has said where their data goes.
function dataRoot() {
  const xdg = process.env.XDG_DATA_HOME;
  if (xdg) {
    return path.join(xdg, "tread");
  }
  if (process.platform === "win32") {
    // %LOCALAPPDATA%, never %APPDATA%: the roaming half of a Windows profile is
    // synced to the domain server, and a machine-specific binary must not
    // travel to a machine it was not built for.
    const local =
      process.env.LOCALAPPDATA || path.join(os.homedir(), "AppData", "Local");
    return path.join(local, "tread");
  }
  if (process.platform === "darwin") {
    return path.join(os.homedir(), "Library", "Application Support", "tread");
  }
  return path.join(os.homedir(), ".local", "share", "tread");
}

function exeName() {
  return process.platform === "win32" ? "tread.exe" : "tread";
}

/// GET `url` and buffer the body, following the redirect GitHub sends to its
/// asset store. Bounded: a body that will not fit in memory is not a release.
function get(url, redirects = 0) {
  return new Promise((resolve, reject) => {
    const req = https.get(
      url,
      { headers: { "user-agent": `tread-npm/${VERSION}` } },
      (res) => {
        const { statusCode, headers } = res;
        if (statusCode >= 300 && statusCode < 400 && headers.location) {
          res.resume();
          if (redirects >= MAX_REDIRECTS) {
            reject(new Error("too many redirects"));
            return;
          }
          resolve(get(new URL(headers.location, url).toString(), redirects + 1));
          return;
        }
        if (statusCode !== 200) {
          res.resume();
          reject(new Error(`HTTP ${statusCode} for ${url}`));
          return;
        }
        const chunks = [];
        let size = 0;
        res.on("data", (c) => {
          size += c.length;
          if (size > MAX_BYTES) {
            req.destroy();
            reject(new Error("response is larger than a release could be"));
            return;
          }
          chunks.push(c);
        });
        res.on("end", () => resolve(Buffer.concat(chunks)));
      }
    );
    req.on("error", reject);
    req.setTimeout(60_000, () => {
      req.destroy();
      reject(new Error("timed out"));
    });
  });
}

/// The SHA-256 the release published for `asset`, as lower-case hex.
///
/// SHA256SUMS is the concatenation of `sha256sum` output, one `<hex>  <name>`
/// line per artifact. An asset missing from it is a release we refuse to trust,
/// not one we install anyway.
function sumFor(sums, asset) {
  for (const line of sums.toString("utf8").split("\n")) {
    const [hex, name] = line.trim().split(/\s+/);
    if (name === asset && /^[0-9a-f]{64}$/.test(hex)) {
      return hex;
    }
  }
  return null;
}

/// Fetch, verify and unpack the binary for this platform into `dir`.
///
/// The write is atomic by rename from a pid-named temporary, so two first runs
/// racing each other both succeed and one of them wins the rename. No lock file
/// is needed and none is left behind if a run is killed mid-download.
async function install(dir, target) {
  const win = process.platform === "win32";
  const asset = `tread-v${VERSION}-${target}${win ? ".zip" : ".tar.gz"}`;
  const base = `https://github.com/${REPO}/releases/download/v${VERSION}`;
  process.stderr.write(`tread: fetching ${asset}\n`);

  const [sums, packed] = await Promise.all([
    get(`${base}/SHA256SUMS`),
    get(`${base}/${asset}`),
  ]);

  const want = sumFor(sums, asset);
  if (!want) {
    throw new Error(`${asset} is not listed in SHA256SUMS`);
  }
  const got = crypto.createHash("sha256").update(packed).digest("hex");
  if (got !== want) {
    throw new Error(`checksum mismatch for ${asset}: expected ${want}, got ${got}`);
  }

  // Only now, with the archive proven to be the one the release published, is
  // anything read out of it.
  const binary = extract(asset, packed, exeName());
  if (!binary) {
    throw new Error(`${asset} does not contain ${exeName()}`);
  }

  fs.mkdirSync(dir, { recursive: true });
  const tmp = path.join(dir, `.tread-${process.pid}`);
  fs.writeFileSync(tmp, binary, { mode: 0o755 });
  fs.renameSync(tmp, path.join(dir, exeName()));
}

/// Drop every version directory but this one, once this one is known good.
function prune(root, keep) {
  let entries = [];
  try {
    entries = fs.readdirSync(root);
  } catch {
    return;
  }
  for (const name of entries) {
    if (name !== keep) {
      fs.rmSync(path.join(root, name), { recursive: true, force: true });
    }
  }
}

/// Hand the terminal to the binary: stdio is inherited, so raw mode, resize and
/// the terminal's own drag-select all reach it untouched, and tread's exit code
/// is this process's exit code.
function run(bin) {
  const r = spawnSync(bin, process.argv.slice(2), { stdio: "inherit" });
  if (r.error) {
    die(`could not run ${bin}: ${r.error.message}`);
  }
  process.exit(r.status === null ? 1 : r.status);
}

async function main() {
  const override = process.env.TREAD_BINARY;
  if (override) {
    if (!fs.existsSync(override)) {
      die(`TREAD_BINARY is set to ${override}, which does not exist`);
    }
    run(override);
  }

  const key = `${process.platform}-${process.arch}`;
  const target = TARGETS[key];
  if (!target) {
    die(
      `no published build for ${key}. Install from source with ` +
        `\`cargo install tread\`, or point TREAD_BINARY at a tread you built.`
    );
  }

  const root = dataRoot();
  const dir = path.join(root, VERSION);
  const bin = path.join(dir, exeName());

  if (!fs.existsSync(bin)) {
    try {
      await install(dir, target);
    } catch (e) {
      die(
        `could not install the ${target} build: ${e.message}\n` +
          `  tread was not downloaded, so nothing unverified has been run.\n` +
          `  Offline or behind a proxy? Install it yourself and set TREAD_BINARY.`
      );
    }
    prune(root, VERSION);
  }

  run(bin);
}

main().catch((e) => die(e.message));
