// The latest GitHub release of a public repository, used only when its tag is on the repository's main branch,
// downloaded and unpacked into <cache>/<tag>/. Shared by sync-artcraft (acaprino/artcraft-web) and sync-sigrid.
// Qamille always takes the latest release of the repository's main, with no pin of its own. The repository is
// public: plain HTTPS, no sign-in (GH_TOKEN or GITHUB_TOKEN, when set, only lifts GitHub's rate limit).
// A release is a manifest.json ({ packages: [{ file, sha256, ... }] }) and the package files it lists. Every
// package is checked against the manifest's SHA-256 before it is unpacked; one release is kept in the cache,
// earlier ones are removed.
// Resolves to { tag, dir }, or null (no network, not on main, not downloaded): the caller then falls back to the
// copy it already has.
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, readdirSync, renameSync, rmSync, writeFileSync } from 'node:fs';
import { basename, join } from 'node:path';

const sha256 = (data) => createHash('sha256').update(data).digest('hex');
// Windows' own tar (bsdtar) reads D:\ paths; Git's GNU tar would take "D:" for a remote host.
const TAR = process.platform === 'win32' ? join(process.env.SystemRoot ?? 'C:\\Windows', 'System32', 'tar.exe') : 'tar';
// Read on every request, so a token set later still counts.
const headers = (accept) => {
  const token = process.env.GH_TOKEN || process.env.GITHUB_TOKEN;
  return { Accept: accept, 'User-Agent': 'github-release', ...(token ? { Authorization: `Bearer ${token}` } : {}) };
};

async function get(url, accept) {
  const response = await fetch(url, { headers: headers(accept), signal: AbortSignal.timeout(120_000) });
  if (!response.ok) throw new Error(`${response.status} ${response.statusText} for ${url}`);
  return response;
}

// A manifest names its packages by file: only a plain file name is trusted, never a path (nothing is then written
// outside the staging folder) and never "." or "..".
export const isPlainFileName = (name) => typeof name === 'string' && name !== '' && name !== '.' && name !== '..' && basename(name) === name;

// Unpacks a release package (an npm-style .tgz: one top folder) into `into`, without that folder; `into` is
// created when it is missing.
export function untar(file, into) {
  mkdirSync(into, { recursive: true });
  execFileSync(TAR, ['-xzf', file, '-C', into, '--strip-components=1'], { stdio: 'ignore' });
}

// repo: "owner/name". cache: the folder that holds one folder per release tag. label: what the log lines call the
// repository. unpack(file, part, pkg): unpacks one downloaded package (file, inside the staging folder `part`; pkg is
// its manifest entry); it is awaited, and its error makes the whole release unusable. The file is removed after it.
// select(manifest, tag), optional: the manifest entries to download, in order (all of manifest.packages by default);
// it throws when the release lacks one the caller needs. api: where GitHub's API is (tests replace it).
export async function latestMainRelease({ repo, cache, label, unpack, select = (manifest) => manifest.packages, log = console.log, api = 'https://api.github.com' }) {
  let release;
  try {
    release = await (await get(`${api}/repos/${repo}/releases/latest`, 'application/vnd.github+json')).json();
  } catch (error) {
    log(`${label}: latest release of ${repo} not reachable (${error.message})`);
    return null;
  }
  const tag = release.tag_name;
  // Only releases of the repository's main: its release workflow tags main, and the tag must be contained in it.
  try {
    const { status } = await (await get(`${api}/repos/${repo}/compare/main...${encodeURIComponent(tag)}`, 'application/vnd.github+json')).json();
    if (status !== 'identical' && status !== 'behind') {
      log(`${label}: ${tag} is not on main (${status}), not used`);
      return null;
    }
  } catch (error) {
    log(`${label}: could not check that ${tag} is on main (${error.message})`);
    return null;
  }
  const dir = join(cache, tag);
  if (!existsSync(join(dir, '.complete'))) {
    const part = `${dir}.part`;
    rmSync(part, { recursive: true, force: true });
    mkdirSync(part, { recursive: true });
    try {
      const asset = (name) => release.assets.find((a) => a.name === name) ?? (() => { throw new Error(`${tag} has no ${name}`); })();
      const download = async (name) => Buffer.from(await (await get(asset(name).url, 'application/octet-stream')).arrayBuffer());
      const manifest = JSON.parse((await download('manifest.json')).toString('utf8'));
      const packages = select(manifest, tag);
      if (!Array.isArray(packages) || packages.length === 0) throw new Error(`the manifest of ${tag} lists no package`);
      for (const pkg of packages) {
        if (!isPlainFileName(pkg?.file)) throw new Error(`the manifest of ${tag} lists ${JSON.stringify(pkg?.file)}, which is not a plain file name`);
      }
      for (const pkg of packages) {
        const data = await download(pkg.file);
        if (sha256(data) !== pkg.sha256) throw new Error(`${pkg.file}: SHA-256 differs from the manifest`);
        const file = join(part, pkg.file);
        writeFileSync(file, data);
        await unpack(file, part, pkg);
        rmSync(file);
      }
      writeFileSync(join(part, '.complete'), `${tag}\n`);
      rmSync(dir, { recursive: true, force: true });
      renameSync(part, dir);
      log(`${label}: ${tag} downloaded from ${repo}`);
    } catch (error) {
      rmSync(part, { recursive: true, force: true });
      log(`${label}: ${tag} could not be downloaded (${error.message})`);
      return null;
    }
  }
  for (const name of readdirSync(cache)) if (name !== tag) rmSync(join(cache, name), { recursive: true, force: true });
  return { tag, dir };
}
