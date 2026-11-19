import type { PreviewKind } from './protocol';

export interface SourceFrame {
  kind: PreviewKind;
  bytes: ArrayBuffer;
}

const KINDS: readonly string[] = ['pdf', 'docx', 'xlsx', 'pptx'];

/** `[u32 LE metadata length][JSON {kind}][document bytes]`, as `attachment_preview_source` sends it. */
export function decodeSourceFrame(payload: ArrayBuffer | Uint8Array<ArrayBuffer>): SourceFrame {
  const bytes = payload instanceof Uint8Array ? payload : new Uint8Array(payload);
  if (bytes.byteLength < 4) throw new Error('Incomplete attachment source frame.');
  const length = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength).getUint32(0, true);
  if (bytes.byteLength < 4 + length) throw new Error('Incomplete attachment source metadata.');
  const metadata = JSON.parse(new TextDecoder().decode(bytes.subarray(4, 4 + length))) as {
    kind?: string;
  };
  if (!metadata.kind || !KINDS.includes(metadata.kind)) {
    throw new Error('Unknown attachment preview type.');
  }
  return { kind: metadata.kind as PreviewKind, bytes: bytes.slice(4 + length).buffer };
}
