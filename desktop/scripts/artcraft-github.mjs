// The latest GitHub release of acaprino/artcraft-web, unpacked like artcraft-web's dist/: <cache>/<tag>/<engine>/.
// Qamille always takes the latest release of
// artcraft-web's main branch (its tag must be on main), with no pin of its own. The repository is public: plain HTTPS, no sign-in (GH_TOKEN or GITHUB_TOKEN, when set, only
// lifts GitHub's rate limit). Every package is checked against the release's manifest.json (SHA-256) before it is
// unpacked; one release is kept in the cache, earlier ones are removed.
// Resolves to { tag, dir }, or null (no network, no release): the caller then falls back to a local artcraft-web.
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, readdirSync, renameSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

export const ARTCRAFT_REPO = 'acaprino/artcraft-web';
export const ARTCRAFT_ENGINES = ['pdfcraft', 'wordcraft', 'gridcraft', 'deckcraft'];

const sha256 = (data) => createHash('sha256').update(data).digest('hex');
// Windows' own tar (bsdtar) reads D:\ paths; Git's GNU tar would take "D:" for a remote host.
const TAR = process.platform === 'win32' ? join(process.env.SystemRoot ?? 'C:\\Windows', 'System32', 'tar.exe') : 'tar';
const token = process.env.GH_TOKEN || process.env.GITHUB_TOKEN;
const headers = (accept) => ({ Accept: accept, 'User-Agent': 'artcraft-github', ...(token ? { Authorization: `Bearer ${token}` } : {}) });

async function get(url, accept) {
  const response = await fetch(url, { headers: headers(accept), signal: AbortSignal.timeout(120_000) });
  if (!response.ok) throw new Error(`${response.status} ${response.statusText} for ${url}`);
  return response;
}

export async function latestArtcraftRelease(cache, log = console.log) {
  let release;
  try {
    release = await (await get(`https://api.github.com/repos/${ARTCRAFT_REPO}/releases/latest`, 'application/vnd.github+json')).json();
  } catch (error) {
    log(`artcraft-web: latest release of ${ARTCRAFT_REPO} not reachable (${error.message})`);
    return null;
  }
  const tag = release.tag_name;
  // Only releases of artcraft-web's main: its release workflow tags main, and the tag must be contained in it.
  try {
    const { status } = await (await get(`https://api.github.com/repos/${ARTCRAFT_REPO}/compare/main...${encodeURIComponent(tag)}`, 'application/vnd.github+json')).json();
    if (status !== 'identical' && status !== 'behind') {
      log(`artcraft-web: ${tag} is not on main (${status}), not used`);
      return null;
    }
  } catch (error) {
    log(`artcraft-web: could not check that ${tag} is on main (${error.message})`);
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
      for (const engine of ARTCRAFT_ENGINES) {
        const pkg = manifest.packages.find((p) => p.engine === engine);
        if (!pkg) throw new Error(`${engine} is not in the manifest of ${tag}`);
        const data = await download(pkg.file);
        if (sha256(data) !== pkg.sha256) throw new Error(`${pkg.file}: SHA-256 differs from the manifest`);
        const file = join(part, pkg.file);
        writeFileSync(file, data);
        mkdirSync(join(part, engine));
        execFileSync(TAR, ['-xzf', file, '-C', join(part, engine), '--strip-components=1'], { stdio: 'ignore' });
        // The npm package is dist/<engine> plus these two.
        for (const extra of ['package.json', 'README.md']) rmSync(join(part, engine, extra), { force: true });
        rmSync(file);
      }
      writeFileSync(join(part, '.complete'), `${tag}\n`);
      rmSync(dir, { recursive: true, force: true });
      renameSync(part, dir);
      log(`artcraft-web: ${tag} downloaded from ${ARTCRAFT_REPO}`);
    } catch (error) {
      rmSync(part, { recursive: true, force: true });
      log(`artcraft-web: ${tag} could not be downloaded (${error.message})`);
      return null;
    }
  }
  for (const name of readdirSync(cache)) if (name !== tag) rmSync(join(cache, name), { recursive: true, force: true });
  return { tag, dir };
}
