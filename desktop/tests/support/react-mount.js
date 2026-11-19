// Mounts one React component of the app in a test page, with the same shape Svelte's `mount` had, so
// a fixture keeps reading `mount(Component, { target, props })`:
//
//   const { mount, unmount } = await import('/tests/support/react-mount.js');
//   const { default: Component } = await import('/src/components/Calendar.tsx');
//   const app = mount(Component, { target: document.querySelector('#fixture'), props: { notify } });
//   app.update({ ...app.props, dark: true }); // re-render with new props
//   unmount(app);
//
// The module goes through the Vite server like any app module, so `react` and `react-dom/client`
// resolve to the very copies the app uses. It loads @vitejs/plugin-react's preamble first: a stub page
// answered by page.route does not get the one Vite writes into index.html, and every .tsx module
// calls the refresh hooks it defines. Import this module before the component.
import '@vitejs/plugin-react/preamble';
import { createElement } from 'react';
import { flushSync } from 'react-dom';
import { createRoot } from 'react-dom/client';
import { useLanguage } from '../../src/lib/i18n.ts';

export { flushSync };

// Subscribes to the interface language, as App.tsx's root does: a setLanguageChoice() made after the
// mount re-renders the component in the new language, as it did with Svelte's reactive i18n.
function Host({ component, props }) {
  useLanguage();
  return createElement(component, props);
}

/** Renders `Component` with `props` into `target`, synchronously (as Svelte's mount did). */
export function mount(Component, { target, props = {} }) {
  const root = createRoot(target);
  const app = {
    props,
    update(next) {
      app.props = next;
      flushSync(() => root.render(createElement(Host, { component: Component, props: next })));
    },
    unmount() {
      root.unmount();
    },
  };
  app.update(props);
  return app;
}

export function unmount(app) {
  app.unmount();
}
