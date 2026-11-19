// support/react-mount.js comes first: it sets up React's refresh preamble before the .tsx loads.
import { mount } from './support/react-mount.js';
import { setLanguageChoice } from '../src/lib/i18n.ts';
import AttachmentPanel from '../src/components/AttachmentPanel.tsx';

setLanguageChoice('en');
mount(AttachmentPanel, {
  target: document.querySelector('#fixture'),
  props: {
    emailId: 42,
    attachment: { id: 5, name: 'postacert.eml', size: 4096, mime: 'message/rfc822', path: null, inline_text: null },
    notify: (message, error) => window.panelNotices.push({ message, error: !!error }),
    dark: false,
    close: () => {
      window.panelClosed += 1;
    },
  },
});
