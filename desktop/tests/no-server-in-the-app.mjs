// The app ships without qamille-server.exe; the texts point to the server on the PC (Docker).
// Run: node tests/no-server-in-the-app.mjs   (from desktop/)
import fs from 'node:fs';
import path from 'node:path';

const root = path.resolve(import.meta.dirname, '..', '..');
const read = (file) => fs.readFileSync(path.join(root, file), 'utf8').replace(/\r\n/g, '\n');
const bat = read('build-release.bat');
const readme = read('README.md');
const failures = [];
const check = (name, ok) => {
  if (ok) console.log(`ok   ${name}`);
  else {
    failures.push(name);
    console.error(`FAIL ${name}`);
  }
};

check('the_app_ships_without_the_server: tauri.professional.conf.json has no resources', JSON.parse(read('desktop/src-tauri/tauri.professional.conf.json')).bundle.resources === undefined);
check('build-release.bat builds and copies no qamille-server.exe', !/cargo build --manifest-path server|copy \/y "%REL%\\qamille-server\.exe"/.test(bat));
check('build-release.bat removes the old server exe from the install folder', /del .*%INSTALL%\\qamille-server\.exe/.test(bat));
check('the README no longer promises local embeddings or local Whisper', !/Embeddings run locally|local Whisper/.test(readme));
check('the docs say the server can run on the PC with Docker', /Docker/.test(read('docs/edizioni.md')) && /server-sul-pc/.test(readme));

if (failures.length) {
  console.error(`\n${failures.length} failed`);
  process.exit(1);
}
