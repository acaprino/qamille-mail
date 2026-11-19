export type PreviewKind = 'pdf' | 'docx' | 'xlsx' | 'pptx';

export type WorkerCommand =
  | { op: 'open'; kind: PreviewKind; bytes: ArrayBuffer }
  | { op: 'page'; page: number; scale: number }
  | { op: 'close' };

export type WorkerRequest = WorkerCommand & { id: number };

export type WorkerReply =
  | { id: number; ok: true; pageCount?: number; warnings?: string[]; image?: ArrayBuffer }
  | { id: number; ok: false; error: string };
