// When Playwright says an element is never stable, this says why: its box over six frames, and what moves on the way up (a transform, an
// animation or a transition running on it or on an ancestor).
export async function explainMotion(locator) {
  return locator.evaluate(async (element) => {
    const rects = [];
    for (let frame = 0; frame < 6; frame += 1) {
      const r = element.getBoundingClientRect();
      rects.push(`${r.x.toFixed(2)},${r.y.toFixed(2)},${r.width.toFixed(2)}x${r.height.toFixed(2)}`);
      await new Promise((done) => requestAnimationFrame(done));
    }
    const chain = [];
    for (let node = element; node; node = node.parentElement) {
      const style = getComputedStyle(node);
      const running = (node.getAnimations?.() ?? []).map((a) => `${a.animationName ?? a.transitionProperty ?? 'animation'}:${a.playState}`);
      if (style.transform !== 'none' || running.length) {
        chain.push(`${node.tagName.toLowerCase()}.${String(node.className).slice(0, 50)} transform=${style.transform} running=${running.join('|')}`);
      }
    }
    return { rects, chain };
  });
}
