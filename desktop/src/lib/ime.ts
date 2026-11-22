// The Enter (or Escape) that confirms or drops an IME candidate (Japanese, Chinese, Korean input)
// belongs to the IME, not to the field's Enter: no submit, no search (docs/aspetto-nativo.md, rule
// 24). Chromium marks it isComposing; keyCode 229 also covers the WebKit order, where
// compositionend comes first.
export function imeComposing(event: Pick<KeyboardEvent, 'isComposing' | 'keyCode'>): boolean {
  return event.isComposing || event.keyCode === 229;
}
