import { useMemo, useRef, useState } from 'react';
import { call, errorText } from '../lib/api';
import { tr } from '../lib/i18n';
import { messageDocument } from '../lib/message-html';
import { installBrowserGuards } from '../lib/browser-guards';
import './MessageHtml.css';

// The keys that read and select stay in the frame, where they scroll the message and move the selection.
const READING_KEYS = new Set(['ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight', 'PageUp', 'PageDown', 'Home', 'End', ' ', 'Tab']);

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
    const anchor = target?.closest?.('a[href]') as HTMLAnchorElement | null;
    if (!anchor) return;
    const href = anchor.getAttribute('href') ?? '';
    if (href.startsWith('#')) return;
    // The frame never navigates; real links open in the system browser.
    event.preventDefault();
    if (!/^(https?:|mailto:)/i.test(href)) return;
    call('open_link', { url: anchor.href }).catch((e) => notifyRef.current(errorText(e), true));
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
