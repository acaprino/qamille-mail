import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
import { existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

// Sigrid is the copy `npm run sigrid:sync` (predev, prebuild) puts in vendor/sigrid, never an npm package: the alias
// reaches its entry on the exact name, and React stays Qamille's one copy. Without the copy nothing starts, so a dev
// server or a build never goes ahead without Sigrid (AGENTS.md, «Sigrid»).
const sigrid = (path: string) => fileURLToPath(new URL(`./vendor/sigrid/${path}`, import.meta.url));
if (!existsSync(sigrid('provenance.json'))) {
  throw new Error('Sigrid manca: `npm run sigrid:sync` in `desktop/`. (Sigrid is missing: run `npm run sigrid:sync` in `desktop/`.)');
}

export default defineConfig({
  plugins: [react(), tailwindcss()],
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  resolve: {
    alias: [
      { find: '@', replacement: fileURLToPath(new URL('./src', import.meta.url)) },
      { find: /^@acaprino\/sigrid$/, replacement: sigrid('dist/index.js') },
    ],
    dedupe: ['react', 'react-dom'],
  },
  build: {
    target: 'es2022',
    // The meeting reminder chip has its own window and page.
    rolldownOptions: {
      input: {
        main: fileURLToPath(new URL('./index.html', import.meta.url)),
        reminder: fileURLToPath(new URL('./reminder.html', import.meta.url)),
      },
    },
  },
  envPrefix: ['VITE_'],
});
