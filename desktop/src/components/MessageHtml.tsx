import { useMemo, useRef, useState } from 'react';
import { tr } from '../lib/i18n';
import { messageDocument } from '../lib/message-html';
import { externalAddress, fragmentOf, installBrowserGuards, linkHref } from '../lib/browser-guards';
import { openExternal } from '../lib/external-link';
import './MessageHtml.css';

// The keys that read and select stay in the frame, where they scroll the message and move the selection.
const READING_KEYS = new Set(['ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight', 'PageUp', 'PageDown', 'Home', 'End', ' ', 'Tab']);

/** A link to a part of the message, as the browser would follow it: the element whose `id` is the
 *  fragment, else the `a` it names with `name`, else the top (an empty fragment, or `top`). The
 *  frame is as tall as its message, so this scrolls the reader around it. */
function scrollToFragment(doc: Document, fragment: string) {
  let decoded = fragment;
  try {
    decoded = decodeURIComponent(fragment);
  } catch {
    // Not percent-encoded text: the fragment as written.
  }
  const named = (name: string) => Array.from(doc.getElementsByName(name)).find((element) => element.localName === 'a');
  const top = fragment === '' || decoded.toLowerCase() === 'top' ? doc.documentElement : undefined;
  const target = doc.getElementById(fragment) ?? doc.getElementById(decoded) ?? named(fragment) ?? named(decoded) ?? top;
  target?.scrollIntoView();
}

export type MessageHtmlProps = {
  html: string;
  notify: (message: string, error?: boolean) => void;
  dark: boolean;
};

export default function MessageHtml({ html, notify, dark }: MessageHtmlProps) {
  // Remote content is allowed for one message only: a new `html` starts blocked again.
  const [remoteFor, setRemoteFor] = useState<string | null>(null);
  const showRemote = remoteFor === html;
  const frame = useRef<HTMLIFrameElement>(null);
  const [height, setHeight] = useState(160);
  const prepared = useMemo(() => messageDocument(html, showRemote, dark), [html, showRemote, dark]);
  const notifyRef = useRef(notify);
  notifyRef.current = notify;

  function measure() {
    const doc = frame.current?.contentDocument;
    if (!doc?.documentElement) return;
    setHeight(Math.max(80, Math.ceil(doc.documentElement.scrollHeight)));
  }
  function openLink(event: MouseEvent) {
    const target = event.target as Element | null;
    const href = target ? linkHref(target) : null;
    if (!target || href === null) return;
    // The frame never navigates, whatever kind of link was clicked (`a`, an image map's `area`, an
    // SVG link). A link to a part of the message scrolls to it: left to the frame it would resolve
    // against the app's own address, and the frame would end in the browser's error page. An absolute
    // web or mail address opens in the system browser; anything else does nothing, a relative address
    // included: it would only resolve against the app's own.
    event.preventDefault();
    const fragment = fragmentOf(href);
    if (fragment !== null) {
      scrollToFragment(target.ownerDocument, fragment);
      return;
    }
    const address = externalAddress(href);
    if (address === null) return;
    void openExternal({ preventDefault: () => event.preventDefault(), currentTarget: { href: address } }, (message) =>
      notifyRef.current(message, true),
    );
  }
  // A key pressed in the message body stays in the frame and never reaches the app's shortcuts
  // (App.tsx keyboard(): zoom, Ctrl+K, Ctrl+Z, N, R, E). A copy goes to the frame element, so it
  // bubbles through the page like any key and the app's own checks on the target still apply.
  function forwardKey(event: KeyboardEvent) {
    const host = frame.current;
    const command = event.ctrlKey || event.metaKey;
    if (!host || READING_KEYS.has(event.key) || (command && ['a', 'c', 'insert'].includes(event.key.toLowerCase()))) return;
    const copy = new KeyboardEvent('keydown', {
      key: event.key,
      code: event.code,
      ctrlKey: event.ctrlKey,
      shiftKey: event.shiftKey,
      altKey: event.altKey,
      metaKey: event.metaKey,
      repeat: event.repeat,
      bubbles: true,
      cancelable: true,
    });
    host.dispatchEvent(copy);
    if (copy.defaultPrevented) event.preventDefault();
  }
  function loaded() {
    const doc = frame.current?.contentDocument;
    if (!doc) return;
    doc.addEventListener('click', openLink);
    doc.addEventListener('keydown', forwardKey);
    // Images and fonts change the height after the first layout.
    doc.addEventListener('load', measure, true);
    const view = frame.current?.contentWindow as (Window & typeof globalThis) | null;
    // The page's guards never see the frame's events: mouse Back would show the previous message
    // (each srcdoc is a history entry), a dropped file would load in the frame.
    if (view) installBrowserGuards(view);
    if (view?.ResizeObserver) new view.ResizeObserver(measure).observe(doc.documentElement);
    measure();
  }

  return (
    <div className="message-body html-body">
      {prepared.remote && !showRemote && (
        <div className="remote-bar" role="status">
          <span>{tr('Immagini remote bloccate per proteggere la privacy.', 'Remote images blocked to protect your privacy.')}</span>
          <button type="button" onClick={() => setRemoteFor(html)}>
            {tr('Mostra immagini', 'Show images')}
          </button>
        </div>
      )}
      <iframe
        ref={frame}
        title={tr('Contenuto del messaggio', 'Message content')}
        sandbox="allow-same-origin"
        srcDoc={prepared.document}
        style={{ height: `${height}px` }}
        onLoad={loaded}
      ></iframe>
    </div>
  );
}
