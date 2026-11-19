// support/react-mount.js comes first: it sets up React's refresh preamble before the .tsx loads.
import { mount, unmount } from './support/react-mount.js';
import { setLanguageChoice } from '../src/lib/i18n.ts';
import AttachmentPreview from '../src/components/AttachmentPreview.tsx';

const parameters = new URLSearchParams(window.location.search);
setLanguageChoice('en');

const component = mount(AttachmentPreview, {
  target: document.querySelector('#fixture'),
  props: {
    emailId: Number(parameters.get('emailId') || 42),
    attachment: JSON.parse(parameters.get('attachment') || '{}'),
    notify: (message, error) => window.previewNotices.push({ message, error: !!error }),
    close: () => {
      window.previewParentClosed += 1;
    },
  },
});

// A new attachment for the same preview, rendered before this returns (as Svelte's flushSync did).
window.setAttachment = (next) => component.update({ ...component.props, attachment: next });
window.unmountPreview = () => unmount(component);
