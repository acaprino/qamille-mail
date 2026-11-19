// What Svelte's `tick()` gave: a promise that settles once React has committed the state updates
// queued before it, so code after `await tick()` sees the new DOM.
// It waits for the next frame (the browser has laid the page out), but never for longer than a short
// timer: a minimized or hidden window gets no animation frames, and the code after the await must still run.
const FRAME_FALLBACK_MS = 50;

export function tick(): Promise<void> {
  return new Promise((resolve) => {
    // A macrotask runs after React's batched render of the current event.
    setTimeout(() => {
      let done = false;
      const finish = () => {
        if (done) return;
        done = true;
        clearTimeout(timer);
        resolve();
      };
      const timer = setTimeout(finish, FRAME_FALLBACK_MS);
      if (typeof document === 'undefined' || document.visibilityState === 'hidden') finish();
      else requestAnimationFrame(finish);
    }, 0);
  });
}
