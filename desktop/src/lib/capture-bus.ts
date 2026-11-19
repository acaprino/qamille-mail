// The recording state, shared. MeetingRecordingIndicator is the only code that polls
// `meeting_capture_status`; it publishes every snapshot it reads and the Meetings page listens,
// so the backend is asked once a second, not once a second per reader.
//
// Kept apart from meetings.ts (which pulls in Tauri, the sample data and the i18n runes) so plain
// node can import it in tests. meetings.ts re-exports all of it.
import type { CaptureSnapshot } from './meetings';

export const emptyCapture = (): CaptureSnapshot => ({
  meeting_id: null,
  state: 'idle',
  elapsed_ms: 0,
  saved_ms: 0,
  microphone_level: 0,
  system_level: 0,
  chunks: [],
  error: null,
  warnings: [],
  missing_chunks: [],
  incomplete: false,
  gaps: [],
  requires_receipts: false,
  unverified_chunks: [],
});

let latest: CaptureSnapshot = emptyCapture();
const captureListeners = new Set<(snapshot: CaptureSnapshot) => void>();

/** The snapshot published last, or an idle one before the first. A copy: change it freely. */
export const lastCapture = (): CaptureSnapshot => structuredClone(latest);

/** Hears every published snapshot, each its own copy. Returns the function that stops listening. */
export function subscribeCapture(listener: (snapshot: CaptureSnapshot) => void): () => void {
  captureListeners.add(listener);
  return () => {
    captureListeners.delete(listener);
  };
}

export function publishCapture(snapshot: CaptureSnapshot): void {
  latest = structuredClone(snapshot);
  for (const listener of [...captureListeners]) {
    // The publisher is a polling loop: a faulty listener must not look like a failed poll,
    // nor keep the others from hearing it.
    try {
      listener(structuredClone(latest));
    } catch (error) {
      console.error('Capture listener failed', error);
    }
  }
}
