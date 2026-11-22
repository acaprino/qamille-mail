// support/react-mount.js comes first: it sets up React's refresh preamble before the .tsx loads.
import { mount } from './support/react-mount.js';
import { setLanguageChoice } from '../src/lib/i18n.ts';
import UpdateNotice from '../src/components/UpdateNotice.tsx';

const parameters = new URLSearchParams(window.location.search);
setLanguageChoice(parameters.get('lang') === 'it' ? 'it' : 'en');
if (parameters.get('theme') === 'dark') document.documentElement.dataset.theme = 'dark';
window.noticeErrors = [];

mount(UpdateNotice, {
  target: document.querySelector('#fixture'),
  props: { onerror: (message) => window.noticeErrors.push(message) },
});
