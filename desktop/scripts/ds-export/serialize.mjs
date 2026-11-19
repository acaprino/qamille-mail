// Functions that run inside the page: they turn live DOM into static markup for a preview card.

// The previews allow no iframe, so an HTML mail becomes a plain block carrying the same body rules the
// app's message-html.ts gives its sandboxed frame.
export const FLATTEN_IFRAMES = `(function (root, live) {
  var PROPS = ['display','box-sizing','margin-top','margin-right','margin-bottom','margin-left','padding-top','padding-right','padding-bottom','padding-left','font-family','font-size','font-weight','font-style','line-height','letter-spacing','color','background-color','border-top-width','border-right-width','border-bottom-width','border-left-width','border-top-style','border-right-style','border-bottom-style','border-left-style','border-top-color','border-right-color','border-bottom-color','border-left-color','border-top-left-radius','border-top-right-radius','border-bottom-left-radius','border-bottom-right-radius','width','max-width','text-align','text-decoration-line','list-style-type','white-space','overflow-wrap'];
  function copy(from, to, win) {
    var cs = win.getComputedStyle(from);
    var out = [];
    PROPS.forEach(function (n) {
      if (n === 'width' && (from.tagName === 'IMG' || cs.display === 'inline')) return;
      out.push(n + ':' + cs.getPropertyValue(n));
    });
    to.setAttribute('style', out.join(';'));
    to.removeAttribute('class');
    if (to.tagName === 'IMG') to.setAttribute('src', 'about:blank');
    for (var i = 0; i < from.children.length; i++) if (to.children[i]) copy(from.children[i], to.children[i], win);
  }
  [...root.querySelectorAll('iframe')].forEach(function (f, i) {
    var src = live[i];
    if (!src) return;
    var d = src.contentDocument;
    var box = document.createElement('div');
    box.className = 'dc-mail ' + (f.getAttribute('class') || '');
    box.setAttribute('style', 'height:' + src.getBoundingClientRect().height + 'px;width:100%;overflow:hidden;background:' + src.contentWindow.getComputedStyle(d.documentElement).backgroundColor);
    var body = d.body.cloneNode(true);
    copy(d.body, body, src.contentWindow);
    body.querySelectorAll('style,script').forEach(function (n) { n.remove(); });
    box.innerHTML = '<div style="' + body.getAttribute('style').replace(/"/g, '&quot;') + '">' + body.innerHTML + '</div>';
    f.replaceWith(box);
  });
})`;

// One component's real DOM with its ancestors' classes kept as box-less wrappers, so descendant selectors
// still match without the ancestors' layout. Each selector is a string or { sel, nth, text }.
// `prefix` names the ids of this capture when several captures end up in one card (see normalizeIds).
export function serializeFragment({ selectors, flatten, prefix = '' }) {
  const vis = (e) => { const r = e.getBoundingClientRect(); return r.width > 0 && r.height > 0; };
  document.querySelectorAll('input,textarea,select,option').forEach((e) => {
    if (e.tagName === 'INPUT') { e.setAttribute('value', e.value); if (e.checked) e.setAttribute('checked', ''); else e.removeAttribute('checked'); }
    if (e.tagName === 'TEXTAREA') e.textContent = e.value;
    if (e.tagName === 'OPTION') { if (e.selected) e.setAttribute('selected', ''); else e.removeAttribute('selected'); }
  });
  [...document.body.querySelectorAll('*')].forEach((e) => { if (e.scrollTop > 0 || e.scrollLeft > 0) e.setAttribute('data-st', `${Math.round(e.scrollTop)},${Math.round(e.scrollLeft)}`); });
  const parts = [];
  const missing = [];
  const clones = [];
  for (const sel of selectors) {
    const spec = typeof sel === 'string' ? { sel } : sel;
    let els = [...document.querySelectorAll(spec.sel)].filter(vis);
    if (spec.text) els = els.filter((e) => e.innerText.trim().startsWith(spec.text));
    const el = els[spec.nth || 0];
    if (!el) { missing.push(spec.sel); continue; }
    const r = el.getBoundingClientRect();
    const cs = getComputedStyle(el);
    const clone = el.cloneNode(true);
    (0, eval)(flatten)(clone, [...el.querySelectorAll('iframe')]);
    // A blob URL names this page and a random id: nothing a card could play, and different at every run.
    [clone, ...clone.querySelectorAll('*')].forEach((e) => {
      for (const a of ['src', 'href']) if ((e.getAttribute(a) || '').startsWith('blob:')) e.removeAttribute(a);
    });
    const extra = [`width:${Math.ceil(r.width)}px`, `height:${Math.ceil(r.height)}px`, 'flex:none', 'margin:0'];
    // A floating part (a list, a notice) is placed by the browser with offsets, transforms and the `translate` and
    // `scale` properties; in a card it sits in the flow.
    if (cs.position === 'fixed' || cs.position === 'absolute') extra.push('position:relative', 'inset:auto', 'transform:none', 'translate:none', 'scale:none');
    clone.setAttribute('style', `${clone.getAttribute('style') || ''};${extra.join(';')}`);
    let open = '';
    let close = '';
    for (let p = el.parentElement; p && p !== document.body; p = p.parentElement) {
      const cls = p.getAttribute('class');
      if (!cls) continue;
      const tag = p.tagName.toLowerCase() === 'dialog' ? 'div' : p.tagName.toLowerCase();
      open = `<${tag} class="${cls}" style="display:contents">${open}`;
      close = `${close}</${tag}>`;
    }
    clones.push(clone);
    parts.push({ open, close, w: Math.ceil(r.width), h: Math.ceil(r.height) });
  }

  // The invisible strips Ark's toasts carry to keep the pointer on a stack of notices (scaled well outside the
  // notice, so that they make a card scroll) are not part of what a notice looks like.
  clones.forEach((c) => c.querySelectorAll('[data-ghost]').forEach((g) => g.remove()));
  // What Ark writes for the page around an open dialog is not part of the part: the clone is taken alone.
  const roots = clones.flatMap((c) => [c, ...c.querySelectorAll('*')]);
  for (const e of roots) {
    if (e.hasAttribute('data-aria-hidden')) { e.removeAttribute('data-aria-hidden'); e.removeAttribute('aria-hidden'); }
    e.removeAttribute('inert');
    // Whether another layer is open is the harness's business, not the part's.
    e.removeAttribute('data-has-nested');
    e.removeAttribute('data-nested');
    e.removeAttribute('data-ownedby');
    // Set a moment after a toast appears: whether the capture caught it before or after is not its look.
    e.removeAttribute('data-mounted');
  }
  // Ark and Zag give each part an id from a counter (`select::r3:content`), and the parts point at each other
  // through aria-controls, aria-labelledby and the like. The counter depends on what mounted first: renumber
  // the ids by order of appearance, and drop a reference to an element that is not in the capture.
  const ids = new Map();
  for (const e of roots) if (e.id) ids.set(e.id, `ds-${prefix}${ids.size + 1}`);
  for (const e of roots) {
    if (e.id) e.setAttribute('id', ids.get(e.id));
    for (const a of ['for', 'aria-controls', 'aria-labelledby', 'aria-describedby', 'aria-owns', 'aria-activedescendant', 'list', 'headers']) {
      const v = e.getAttribute(a);
      if (!v) continue;
      const kept = v.split(/\s+/).filter((t) => ids.has(t)).map((t) => ids.get(t));
      if (kept.length) e.setAttribute(a, kept.join(' ')); else e.removeAttribute(a);
    }
  }
  parts.forEach((p, i) => { p.html = p.open + clones[i].outerHTML + p.close; });
  const html = parts.map((p) => p.html).join('').replace(/<!--[\s\S]*?-->/g, '').replace(/ data-svelte-h="[^"]*"/g, '');
  return { parts: parts.map(({ w, h }) => ({ w, h })), html, missing };
}
