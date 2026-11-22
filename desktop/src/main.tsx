import { MotionConfig } from 'motion/react';
import { createRoot } from 'react-dom/client';
import App from './App';
import './styles.css';
// Last: the Contrast-theme rules must win over every page rule (docs/aspetto-nativo.md).
import './ds/styles/forced-colors.css';
import { appearancePreference, resolvedAppearance, systemPrefersDark } from './lib/appearance';
import { call, desktop } from './lib/api';
import { installBrowserGuards } from './lib/browser-guards';
import { SigridRoot } from './ui/SigridRoot';

document.documentElement.dataset.theme = resolvedAppearance(appearancePreference(), systemPrefersDark());
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
// SigridRoot: Sigrid's words and motion. MotionConfig: every Motion animation of Qamille's own (Tabs, NavList, Apple
// Switch, rows) honours reduced motion, until the app's motion comes from Sigrid too.
createRoot(document.getElementById('app')!, { onUncaughtError: pageFailed }).render(
  <SigridRoot>
    <MotionConfig reducedMotion="user">
      <App />
    </MotionConfig>
  </SigridRoot>,
);
