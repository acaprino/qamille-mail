// Follows the meetings that are being processed, for the Meetings page. A page at rest runs nothing
// (INV-005 of ADR-0007); this is the one timer it has, and it exists only while something is going
// on: a transcription or a summary waiting or running, or a moment where one is about to be queued.
// Plain TypeScript with no screen, no Svelte and no Tauri: what it reads, where it puts the answer
// and how the list is read again come in through `FollowerDeps`, so `node --test` can drive it.
//
// How it works. The shell tells it which meetings have a job (`sync`) and which recording just
// ended (`settle`). Every second (`FOLLOW_MS`) it reads those meetings in full, never the list, and
// hands each answer to `apply`. A meeting that has no job left makes the list be read once
// (`reload`), then it is not followed any more, and the timer stops when nothing is followed.
//
// The grace. The service does not queue the next job in the instant the previous one ends: after a
// recording it queues the transcription a moment after the stop, and after a transcription it
// queues the summary once the worker's lease is gone (and only if an assistant is set up). A tick
// that lands in that gap sees a meeting with no job and, taken at its word, would stop following
// it and leave the screen on "Genera sintesi" while the summary is being written. So a meeting
// that was just stopped, or whose transcription just ended with no summary after it, is kept for
// `SETTLE_TICKS` more ticks: a job that appears is followed from then on, and when none does the
// list is read once and the meeting is let go.
import type { MeetingDetail } from './meetings';
import { awaitsSummary, hasActiveJob } from './meetings-view.ts';

/** How often the meetings in flight are read. */
export const FOLLOW_MS = 1000;
/** Ticks in a row with a failed read before the follower stops trying and says so. */
export const FOLLOW_FAILURES = 3;
/** Ticks a meeting is kept after its recording stopped or its transcription ended, waiting for the next job. */
export const SETTLE_TICKS = 3;

export interface FollowerDeps {
  /** One meeting, in full. */
  read(id: number): Promise<MeetingDetail>;
  /** A meeting read in full: the screen follows (its row, and the detail if it is the open one). */
  apply(detail: MeetingDetail): void;
  /** The list must be read again, once. */
  reload(): Promise<void> | void;
  /** Changes whenever the page takes meetings from somewhere else; an answer older than that is dropped. */
  stamp(): number;
  /** The reads failed `FOLLOW_FAILURES` times in a row; the follower stops. */
  giveUp(error: unknown): void;
  /** True while nobody is looking at the window: the tick is skipped. */
  hidden?(): boolean;
}

/** A read that failed because the meeting does not exist (any more). */
const isGone = (error: unknown): boolean =>
  typeof error === 'object' && error !== null && (error as { code?: unknown }).code === 'not_found';

export class MeetingFollower {
  readonly #deps: FollowerDeps;
  /** The meetings the shell sees with a job queued or running. */
  #watch = new Set<number>();
  /** Meetings waiting for their next job, and the ticks they have left. */
  readonly #settle = new Map<number, number>();
  #timer: ReturnType<typeof setInterval> | undefined;
  #busy = false;
  #failures = 0;
  #key = '';
  #gaveUp = '';
  #disposed = false;

  constructor(deps: FollowerDeps) {
    this.#deps = deps;
  }

  /** The timer is on. */
  get running(): boolean {
    return this.#timer !== undefined;
  }
  /** The meetings read at every tick, ascending. */
  get following(): number[] {
    return this.#ids();
  }

  /** The meetings that have a job right now. The shell says it again whenever they change. */
  sync(ids: number[]): void {
    this.#watch = new Set(ids);
    this.#refresh();
  }

  /** A recording of this meeting just ended: its transcription is about to be queued. */
  settle(id: number): void {
    if (this.#disposed) return;
    if (!this.#settle.has(id)) this.#settle.set(id, SETTLE_TICKS);
    this.#refresh();
  }

  /** One reading of the meetings in flight. The timer calls it; tests call it by hand. */
  async tick(): Promise<void> {
    if (this.#busy || this.#disposed || this.#deps.hidden?.()) return;
    const ids = this.#ids();
    if (!ids.length) return;
    this.#busy = true;
    const stamp = this.#deps.stamp();
    try {
      const results = await Promise.all(
        ids.map(async (id) => {
          try {
            return { id, detail: await this.#deps.read(id), error: null };
          } catch (error) {
            return { id, detail: null, error };
          }
        }),
      );
      // The page took meetings from somewhere newer while these were on their way: they are older.
      if (this.#disposed || stamp !== this.#deps.stamp()) return;
      // A meeting that is gone (deleted while it was followed) is not a failing read: it is let
      // go, and the list is read once so its row goes too. It must not count towards giving up.
      let reload = false;
      for (const { id, error } of results)
        if (isGone(error)) {
          this.#watch.delete(id);
          this.#settle.delete(id);
          reload = true;
        }
      const failed = results.find((result) => result.error !== null && !isGone(result.error));
      if (failed) {
        if (++this.#failures >= FOLLOW_FAILURES) this.#giveUp(failed.error);
      } else this.#failures = 0;
      for (const { id, detail } of results) {
        if (!detail) continue;
        if (hasActiveJob(detail)) {
          // A job is there (the one that was awaited, or the next one): it is followed as usual.
          this.#settle.delete(id);
          this.#watch.add(id);
        } else if (this.#settle.has(id)) {
          const left = (this.#settle.get(id) ?? 0) - 1;
          if (left > 0) this.#settle.set(id, left);
          else {
            // Nothing came: the meeting is what the list says.
            this.#settle.delete(id);
            reload = true;
          }
        } else if (awaitsSummary(detail)) {
          this.#watch.delete(id);
          this.#settle.set(id, SETTLE_TICKS);
        } else {
          this.#watch.delete(id);
          reload = true;
        }
        this.#deps.apply(detail);
      }
      this.#refresh();
      if (reload) await this.#deps.reload();
    } finally {
      this.#busy = false;
    }
  }

  /** Stops the timer for good (the page is gone). */
  dispose(): void {
    this.#disposed = true;
    this.#stop();
    this.#settle.clear();
    this.#watch.clear();
  }

  #ids(): number[] {
    return [...new Set([...this.#watch, ...this.#settle.keys()])].sort((a, b) => a - b);
  }

  /** Starts or stops the timer for what is followed now; a different set starts counting failures again. */
  #refresh(): void {
    if (this.#disposed) return;
    const key = this.#ids().join(',');
    if (key !== this.#key) {
      this.#key = key;
      this.#failures = 0;
      this.#gaveUp = '';
    }
    if (!key || key === this.#gaveUp) this.#stop();
    else if (this.#timer === undefined) this.#timer = setInterval(() => void this.tick(), FOLLOW_MS);
  }

  #stop(): void {
    if (this.#timer !== undefined) clearInterval(this.#timer);
    this.#timer = undefined;
  }

  #giveUp(error: unknown): void {
    // What waited for its next job cannot be read: it is let go, and the set that is left stays
    // given up until it changes.
    this.#settle.clear();
    this.#key = this.#ids().join(',');
    this.#gaveUp = this.#key;
    this.#failures = 0;
    this.#stop();
    this.#deps.giveUp(error);
  }
}
