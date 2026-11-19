// The module mount.mjs loads in the stub page: reads which section to show and mounts the gallery.
// react-mount.js comes first: it sets up React's refresh preamble before Gallery.tsx loads.
const { mount } = await import('./react-mount.js');
const { default: Gallery } = await import('./Gallery.tsx');

const { which, props } = JSON.parse(document.getElementById('gallery-config').textContent);
window.__galleryApp = mount(Gallery, { target: document.getElementById('gallery'), props: { which, props } });
window.__galleryReady = true;
