import { MotionConfig } from 'motion/react';
import { createRoot } from 'react-dom/client';
import App from './App';
import './styles.css';
// Last: the Contrast-theme rules must win over every page rule (docs/aspetto-nativo.md).
import './ds/styles/forced-colors.css';
import { appearancePreference, resolvedAppearance, systemPrefersDark } from './lib/appearance';
import { call, desktop } from './lib/api';
import { installBrowserGuards } from './lib/browser-guards';

document.documentElement.dataset.theme = resolvedAppearance(appearancePreference(), systemPrefersDark());
installBrowserGuards();
// No StrictMode: the root talks to the backend on mount, and its effects must run once.
// MotionConfig: every Motion animation (Tabs, NavList, Apple Switch, rows) honours reduced motion.
createRoot(document.getElementById('app')!).render(
  <MotionConfig reducedMotion="user">
    <App />
  </MotionConfig>,
);
// The page has drawn: without this, src-tauri/src/recovery.rs offers to reload after 15 s.
if (desktop) requestAnimationFrame(() => void call('frontend_ready').catch(() => {}));
