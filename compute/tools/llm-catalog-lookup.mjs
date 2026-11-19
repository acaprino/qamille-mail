#!/usr/bin/env node
// Looks up the facts a catalog entry of compute/src/llm_catalog.rs needs, from the Hugging Face Hub.
// Usage: node compute/tools/llm-catalog-lookup.mjs <repo> <file> [<repo> <file> ...]
// For each pair prints: the commit to pin (`sha`), `gated`, licence tags, the file size and its
// SHA-256 (the LFS oid), the GGUF figures (`block_count`, `head_count_kv`, `key_length`,
// `context_length`), `kv_mb_per_1k`, `ram_mb`, and the first lines of the README and the licence
// file at that revision. Exits with an error when the repository is gated, the file is missing or
// not in LFS. Nothing here downloads a model: only the first 4 MiB of the GGUF header is read.
// The Hub only: a mirror is not consulted (INV-39 pins what the Hub says).

const HUB = "https://huggingface.co";
const [, , ...args] = process.argv;
if (args.length === 0 || args.length % 2 !== 0) {
  console.error("usage: llm-catalog-lookup.mjs <repo> <file> [<repo> <file> ...]");
  process.exit(2);
}

async function json(url) {
  const response = await fetch(url);
  if (!response.ok) throw new Error(`${response.status} for ${url}`);
  return response.json();
}

async function head(url, bytes) {
  const response = await fetch(url, { headers: { Range: `bytes=0-${bytes - 1}` } });
  if (!response.ok && response.status !== 206) throw new Error(`${response.status} for ${url}`);
  return Buffer.from(await response.arrayBuffer());
}

async function text(url, lines) {
  const response = await fetch(url);
  if (!response.ok) return null;
  return (await response.text()).split("\n").slice(0, lines).join("\n");
}

// Minimal GGUF metadata reader: stops as soon as the wanted keys are known.
function ggufFigures(buffer) {
  let at = 0;
  const u32 = () => ((at += 4), buffer.readUInt32LE(at - 4));
  const u64 = () => ((at += 8), Number(buffer.readBigUInt64LE(at - 8)));
  const string = () => {
    const length = u64();
    at += length;
    return buffer.toString("utf8", at - length, at);
  };
  const sizes = { 0: 1, 1: 1, 2: 2, 3: 2, 4: 4, 5: 4, 6: 4, 7: 1, 10: 8, 11: 8, 12: 8 };
  const value = (type) => {
    switch (type) {
      case 0: case 1: case 7: return buffer[(at += 1) - 1];
      case 2: case 3: return buffer.readUInt16LE((at += 2) - 2);
      case 4: return u32();
      case 5: return buffer.readInt32LE((at += 4) - 4);
      case 6: return buffer.readFloatLE((at += 4) - 4);
      case 8: return string();
      case 10: return u64();
      case 11: return Number(buffer.readBigInt64LE((at += 8) - 8));
      case 12: return buffer.readDoubleLE((at += 8) - 8);
      case 9: {
        const inner = u32();
        const count = u64();
        // Short numeric arrays (per-layer figures of Gemma) are kept; the tokenizer's are skipped.
        if (sizes[inner] && count <= 1024) return Array.from({ length: count }, () => value(inner));
        if (sizes[inner]) at += sizes[inner] * count;
        else for (let i = 0; i < count; i++) value(inner);
        return "[array]";
      }
      default: throw new Error(`unknown GGUF type ${type}`);
    }
  };
  if (buffer.toString("latin1", 0, 4) !== "GGUF") throw new Error("not a GGUF file");
  at = 4;
  u32(); // version
  u64(); // tensor count
  const count = u64();
  const found = {};
  for (let i = 0; i < count; i++) {
    const key = string();
    const type = u32();
    const v = value(type);
    found[key] = v;
    // The architecture keys come before the tokenizer, whose arrays are huge.
    if (key.startsWith("tokenizer.")) break;
  }
  const a = found["general.architecture"];
  const layers = found[`${a}.block_count`];
  const kvHeads = found[`${a}.attention.head_count_kv`];
  const heads = found[`${a}.attention.head_count`];
  const keyLength = found[`${a}.attention.key_length`] ?? found[`${a}.embedding_length`] / heads;
  const valueLength = found[`${a}.attention.value_length`] ?? keyLength;
  const keyLengthSwa = found[`${a}.attention.key_length_swa`] ?? keyLength;
  const valueLengthSwa = found[`${a}.attention.value_length_swa`] ?? valueLength;
  const pattern = found[`${a}.attention.sliding_window_pattern`];
  const shared = found[`${a}.attention.shared_kv_layers`] ?? 0;
  // K and V elements per token, summed over the layers that own a cache (the last `shared` layers
  // reuse an earlier one). A sliding-window layer is counted at the full context on purpose: the
  // estimate stays an upper bound, linear in the context.
  let elements = 0;
  for (let i = 0; i < layers - shared; i++) {
    const h = Array.isArray(kvHeads) ? kvHeads[i] : kvHeads;
    const swa = Array.isArray(pattern) && pattern[i] === 1;
    elements += h * (swa ? keyLengthSwa + valueLengthSwa : keyLength + valueLength);
  }
  return { architecture: a, block_count: layers, head_count_kv: Array.isArray(kvHeads) ? kvHeads.join(",") : kvHeads, key_length: keyLength, context_length: found[`${a}.context_length`], kv_elements_per_token: elements };
}

let failed = false;
for (let i = 0; i < args.length; i += 2) {
  const [repo, file] = [args[i], args[i + 1]];
  try {
    const info = await json(`${HUB}/api/models/${repo}?blobs=true`);
    if (info.gated) throw new Error(`${repo} is gated: not allowed in the default catalog`);
    const sibling = info.siblings.find((s) => s.rfilename === file);
    if (!sibling) throw new Error(`${repo} has no file ${file}`);
    if (!sibling.lfs?.sha256) throw new Error(`${file} is not an LFS file`);
    const resolve = `${HUB}/${repo}/resolve/${info.sha}`;
    const figures = ggufFigures(await head(`${resolve}/${file}`, 4 * 1024 * 1024));
    // K and V elements per token x 2 bytes (f16), per 1024 tokens.
    const kvBytes = figures.kv_elements_per_token * 2 * 1024;
    const mib = (n) => Math.ceil(n / 1048576);
    const result = {
      repo,
      revision: info.sha,
      gated: info.gated,
      license_tags: info.tags.filter((t) => t.startsWith("license")),
      file,
      bytes: sibling.size,
      sha256: sibling.lfs.sha256,
      ...figures,
      kv_mb_per_1k: mib(kvBytes),
      ram_mb: mib(sibling.size * 1.1) + 300,
      readme_head: await text(`${resolve}/README.md`, 12),
      license_file_head: await text(`${resolve}/LICENSE`, 3),
    };
    console.log(JSON.stringify(result, null, 2));
  } catch (error) {
    failed = true;
    console.error(`FAILED ${repo} ${file}: ${error.message}`);
  }
}
process.exit(failed ? 1 : 0);
