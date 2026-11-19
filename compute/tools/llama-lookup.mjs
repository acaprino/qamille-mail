#!/usr/bin/env node
// Looks up the facts compute/src/llama.rs pins for the llama.cpp engine (INV-39).
// Usage: node compute/tools/llama-lookup.mjs [<tag>] [--gguf <small model.gguf>]
// Without a tag: lists the releases older than 14 days. With a tag: prints the commit
// (`git ls-remote`), the size and SHA-256 of the source archive of that commit and of the
// Windows CPU x64 archive (compared with the `digest` of the GitHub API), unpacks the Windows
// archive into a NEW EMPTY temp folder outside the repository (downloaded files are untrusted:
// nothing is run from a folder inside the repository), prints the files `llama-server.exe` needs
// (PE import tables, followed from the program through the DLLs of the archive, plus the
// `ggml-cpu-*.dll` backends ggml loads from the program folder), checks that `--help` knows
// every option the supervisor uses, and with `--gguf` starts the server on loopback and checks
// which endpoints ask for the key. Never invents a value: a failed step exits with an error.
import { createHash } from "node:crypto";
import { execFileSync, spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const REPO = "ggml-org/llama.cpp";
const OPTIONS = ["--model", "--host", "--port", "--ctx-size", "--parallel", "--cache-ram", "--no-slots", "--jinja", "--no-webui", "--log-verbosity", "--api-key-file", "LLAMA_API_KEY"];
const args = process.argv.slice(2);
const gguf = args.includes("--gguf") ? args[args.indexOf("--gguf") + 1] : null;
const tag = args.find((a, i) => !a.startsWith("--") && args[i - 1] !== "--gguf");

async function json(url) {
  const response = await fetch(url, { headers: { "User-Agent": "qamille-llama-lookup" } });
  if (!response.ok) throw new Error(`${response.status} for ${url}`);
  return response.json();
}
async function download(url, file) {
  const response = await fetch(url, { redirect: "follow" });
  if (!response.ok) throw new Error(`${response.status} for ${url}`);
  const bytes = Buffer.from(await response.arrayBuffer());
  fs.writeFileSync(file, bytes);
  return { bytes: bytes.length, sha256: createHash("sha256").update(bytes).digest("hex") };
}

// Import tables of a PE file: the DLL names it needs.
function imports(file) {
  const b = fs.readFileSync(file);
  const pe = b.readUInt32LE(0x3c);
  const sections = b.readUInt16LE(pe + 6);
  const optional = pe + 24;
  const directories = optional + (b.readUInt16LE(optional) === 0x20b ? 112 : 96);
  const table = [];
  for (let i = 0; i < sections; i++) {
    const s = optional + b.readUInt16LE(pe + 20) + i * 40;
    table.push({ va: b.readUInt32LE(s + 12), size: Math.max(b.readUInt32LE(s + 8), b.readUInt32LE(s + 16)), raw: b.readUInt32LE(s + 20) });
  }
  const offset = (rva) => {
    const s = table.find((t) => rva >= t.va && rva < t.va + t.size);
    if (!s) throw new Error(`bad rva in ${file}`);
    return rva - s.va + s.raw;
  };
  const name = (rva) => {
    let end = offset(rva);
    const start = end;
    while (b[end]) end++;
    return b.toString("latin1", start, end);
  };
  const names = [];
  const imp = b.readUInt32LE(directories + 8);
  if (imp) for (let o = offset(imp); b.readUInt32LE(o + 12); o += 20) names.push(name(b.readUInt32LE(o + 12)));
  const delayed = b.readUInt32LE(directories + 13 * 8);
  if (delayed) for (let o = offset(delayed); b.readUInt32LE(o + 4); o += 32) names.push(name(b.readUInt32LE(o + 4)));
  return names;
}

if (!tag) {
  const cutoff = Date.now() - 14 * 86400_000;
  const releases = await json(`https://api.github.com/repos/${REPO}/releases?per_page=100&page=3`);
  for (const r of releases.filter((r) => Date.parse(r.published_at) < cutoff).slice(0, 10)) console.log(r.tag_name, r.published_at);
  process.exit(0);
}

const release = await json(`https://api.github.com/repos/${REPO}/releases/tags/${tag}`);
const asset = release.assets.find((a) => a.name === `llama-${tag}-bin-win-cpu-x64.zip`);
if (!asset) throw new Error("no Windows CPU x64 archive in this release");
const ls = execFileSync("git", ["ls-remote", `https://github.com/${REPO}`, `refs/tags/${tag}`, `refs/tags/${tag}^{}`], { encoding: "utf8" }).trim().split("\n").map((l) => l.split("\t"));
const commit = (ls.find((l) => l[1].endsWith("^{}")) ?? ls[0])[0];
if (!/^[0-9a-f]{40}$/.test(commit)) throw new Error("no commit for the tag");

const work = fs.mkdtempSync(path.join(os.tmpdir(), "llama-lookup-"));
const source = await download(`https://github.com/${REPO}/archive/${commit}.tar.gz`, path.join(work, "source.tar.gz"));
const win = await download(asset.browser_download_url, path.join(work, "win.zip"));
if (asset.digest && asset.digest !== `sha256:${win.sha256}`) throw new Error(`the API digest ${asset.digest} differs from ${win.sha256}`);
console.log({ tag, commit, source, windows: { url: asset.browser_download_url, ...win, apiDigest: asset.digest } });

const dir = path.join(work, "win");
fs.mkdirSync(dir);
execFileSync("tar", ["-xf", path.join(work, "win.zip"), "-C", dir]);
const present = new Set(fs.readdirSync(dir));
const needed = new Set();
const walk = (file) => {
  if (needed.has(file)) return;
  needed.add(file);
  for (const dll of imports(path.join(dir, file))) {
    const hit = [...present].find((p) => p.toLowerCase() === dll.toLowerCase());
    if (hit) walk(hit);
  }
};
walk("llama-server.exe");
for (const p of present) if (/^ggml-cpu-.*\.dll$/i.test(p)) needed.add(p);
if (present.has("LICENSE-LLVM-OpenMP")) needed.add("LICENSE-LLVM-OpenMP");
console.log("RUNTIME_FILES", [...needed].sort());

if (process.platform === "win32") {
  const help = execFileSync(path.join(dir, "llama-server.exe"), ["--help"], { encoding: "utf8", maxBuffer: 1 << 24 });
  const missing = OPTIONS.filter((o) => !help.includes(o));
  console.log(missing.length ? { MISSING_OPTIONS: missing } : "every option the supervisor uses is in --help");
  if (gguf) {
    const port = 20000 + Math.floor(Math.random() * 20000);
    const child = spawn(path.join(dir, "llama-server.exe"), ["--model", gguf, "--host", "127.0.0.1", "--port", String(port), "--ctx-size", "128", "--parallel", "1", "--cache-ram", "0", "--no-slots", "--jinja", "--no-webui", "--log-verbosity", "1"], { env: { LLAMA_API_KEY: "probe-key", SystemRoot: process.env.SystemRoot }, stdio: "ignore" });
    try {
      await new Promise((r) => setTimeout(r, 6000));
      for (const [method, route] of [["GET", "/health"], ["GET", "/v1/models"], ["POST", "/tokenize"]]) {
        const status = async (headers) => (await fetch(`http://127.0.0.1:${port}${route}`, { method, headers: { "Content-Type": "application/json", ...headers }, body: method === "POST" ? '{"content":"hi"}' : undefined })).status;
        console.log(method, route, "without key", await status({}), "with key", await status({ Authorization: "Bearer probe-key" }), "wrong key", await status({ Authorization: "Bearer other" }));
      }
    } finally {
      child.kill();
    }
  }
}
console.log("work folder (outside the repository):", work);
