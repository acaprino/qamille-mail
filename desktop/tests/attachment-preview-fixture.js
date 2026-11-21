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
    // The notice's actions are kept, so a test can press them the way a click on the notice would.
    notify: (message, error, actions = []) => {
      window.previewNotices.push({ message, error: !!error, actions: actions.map((action) => action.label) });
      window.previewNoticeActions = actions;
    },
    close: () => {
      window.previewParentClosed += 1;
    },
  },
});

// A new attachment for the same preview, rendered before this returns (as Svelte's flushSync did).
window.setAttachment = (next) => component.update({ ...component.props, attachment: next });
window.unmountPreview = () => unmount(component);
// Presses an action of the latest notice, as the Toaster's button would.
window.runNoticeAction = (label) => window.previewNoticeActions.find((action) => action.label === label).run();
