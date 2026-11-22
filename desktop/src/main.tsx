import { createRoot } from 'react-dom/client';
import App from './App';
import './styles.css';
// Last: the Contrast-theme rules must win over every page rule (docs/aspetto-nativo.md).
import './ds/styles/forced-colors.css';
import { appearancePreference, resolvedAppearance, systemPrefersDark } from './lib/appearance';
import { call, desktop } from './lib/api';
import { installBrowserGuards } from './lib/browser-guards';
import { SigridRoot } from './ui/SigridRoot';

document.documentElement.dataset.scheme = resolvedAppearance(appearancePreference(), systemPrefersDark());
installBrowserGuards();

// With no error boundary, an error while rendering or in an effect makes React remove the whole
// tree, and the window is empty: src-tauri/src/recovery.rs then offers to reload it. (The first
// frame is reported by App's first effect, so a render that throws never passes for ready.)
function pageFailed(error: unknown) {
  console.error(error);
  const message = error instanceof Error ? (error.stack ?? `${error.name}: ${error.message}`) : String(error);
  if (desktop) void call('frontend_failed', { message }).catch(() => {});
}

// No StrictMode: the root talks to the backend on mount, and its effects must run once.
// SigridRoot: Sigrid's words and motion (it honours the system's reduced-motion setting).
createRoot(document.getElementById('app')!, { onUncaughtError: pageFailed }).render(
  <SigridRoot>
    <App />
  </SigridRoot>,
);
