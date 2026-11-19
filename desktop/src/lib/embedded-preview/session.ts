import type { WorkerCommand, WorkerReply, WorkerRequest, PreviewKind } from './protocol';

const OPEN_TIMEOUT_MS = 60_000;
const PAGE_TIMEOUT_MS = 60_000;
const POINTS_PER_INCH = 72;

type Pending = {
  resolve: (reply: Extract<WorkerReply, { ok: true }>) => void;
  reject: (error: Error) => void;
  timer: ReturnType<typeof setTimeout>;
};

/** One document open in its own worker. A stuck or failed document is dropped by killing the worker. */
export class EmbeddedPreviewSession {
  private worker: Worker | null = null;
  private pending = new Map<number, Pending>();
  private next = 0;
  pageCount = 0;
  warnings: string[] = [];

  private start(): Worker {
    if (this.worker) return this.worker;
    const worker = new Worker(new URL('./worker.ts', import.meta.url), { type: 'module' });
    worker.onmessage = (event: MessageEvent<WorkerReply>) => {
      const waiting = this.pending.get(event.data.id);
      if (!waiting) return;
      this.pending.delete(event.data.id);
      clearTimeout(waiting.timer);
      if (event.data.ok) waiting.resolve(event.data);
      else waiting.reject(new Error(event.data.error));
    };
    worker.onerror = (event) => {
      event.preventDefault();
      this.fail(new Error(event.message || 'The preview engine stopped.'));
    };
    this.worker = worker;
    return worker;
  }

  private fail(error: Error) {
    const waiting = [...this.pending.values()];
    this.pending.clear();
    this.worker?.terminate();
    this.worker = null;
    for (const item of waiting) {
      clearTimeout(item.timer);
      item.reject(error);
    }
  }

  private request(
    message: WorkerCommand,
    timeoutMs: number,
    transfer: Transferable[] = [],
  ) {
    const worker = this.start();
    const id = ++this.next;
    return new Promise<Extract<WorkerReply, { ok: true }>>((resolve, reject) => {
      const timer = setTimeout(
        () => this.fail(new Error('The preview took too long and was stopped.')),
        timeoutMs,
      );
      this.pending.set(id, { resolve, reject, timer });
      worker.postMessage({ ...message, id } as WorkerRequest, transfer);
    });
  }

  async open(kind: PreviewKind, bytes: ArrayBuffer): Promise<void> {
    const reply = await this.request({ op: 'open', kind, bytes }, OPEN_TIMEOUT_MS, [bytes]);
    this.pageCount = reply.pageCount ?? 0;
    this.warnings = reply.warnings ?? [];
  }

  async render(page: number, dpi: number): Promise<ArrayBuffer> {
    const reply = await this.request(
      { op: 'page', page: page - 1, scale: dpi / POINTS_PER_INCH },
      PAGE_TIMEOUT_MS,
    );
    if (!reply.image) throw new Error('The preview engine returned no image.');
    return reply.image;
  }

  dispose() {
    this.fail(new Error('Preview closed.'));
  }
}
