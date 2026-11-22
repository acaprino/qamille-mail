import DOMPurify from 'dompurify';

export interface MessageDocument {
  /** Complete document for a sandboxed `srcdoc` frame. */
  document: string;
  /** The message references remote images, backgrounds or fonts. */
  remote: boolean;
}

// Matches remote resources in attributes and CSS: src=, srcset=, background=, url(...).
const REMOTE =
  /\b(?:src|srcset|background|poster)\s*=\s*["']?\s*(?:https?:)?\/\/|url\(\s*["']?\s*(?:https?:)?\/\//i;

const BASE_STYLE = `html{background:#fff;color:#1f2328;overflow-y:hidden;color-scheme:light}
body{margin:0;padding:16px;font:14px/1.5 system-ui,-apple-system,"Segoe UI",sans-serif;overflow-wrap:anywhere;overflow-x:auto}
img{max-width:100%;height:auto}
pre{white-space:pre-wrap}
a{color:#2563eb}
html[data-scheme="dark"]{color-scheme:dark;filter:invert(1) hue-rotate(180deg)}
html[data-scheme="dark"] img,html[data-scheme="dark"] video,html[data-scheme="dark"] svg{filter:invert(1) hue-rotate(180deg)}`;

/**
 * Sanitize received HTML and wrap it for display.
 * Scripts never run: the frame has no `allow-scripts`, and the policy below allows
 * no script source. Remote content is blocked until the reader allows it.
 */
export function messageDocument(html: string, allowRemote: boolean, dark = false): MessageDocument {
  const clean = DOMPurify.sanitize(html, {
    WHOLE_DOCUMENT: true,
    FORBID_TAGS: ['form', 'input', 'button', 'textarea', 'select', 'iframe', 'object', 'embed', 'base', 'meta', 'link'],
    FORBID_ATTR: ['action', 'formaction'],
  });
  const parsed = new DOMParser().parseFromString(clean, 'text/html');
  parsed.documentElement.dataset.scheme = dark ? 'dark' : 'light';
  const images = allowRemote ? 'data: cid: https: http:' : 'data: cid:';
  const fonts = allowRemote ? 'data: https:' : 'data:';
  const policy = parsed.createElement('meta');
  policy.setAttribute('http-equiv', 'Content-Security-Policy');
  policy.setAttribute(
    'content',
    `default-src 'none'; img-src ${images}; style-src 'unsafe-inline'; font-src ${fonts}`,
  );
  const style = parsed.createElement('style');
  style.textContent = BASE_STYLE;
  // The policy must precede everything that can load a resource.
  parsed.head.prepend(policy, style);
  return {
    document: `<!doctype html>${parsed.documentElement.outerHTML}`,
    remote: REMOTE.test(clean),
  };
}
