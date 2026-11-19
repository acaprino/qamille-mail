// Documents generated in memory for the embedded preview tests (no binary fixtures in the repository).
import { deflateSync } from 'node:zlib';

export function samplePdf(lines) {
  const content = ['BT', '/F1 20 Tf', '72 760 Td', '30 TL', ...lines.map((line, i) => `${i ? 'T* ' : ''}(${line}) Tj`), 'ET'].join('\n');
  const stream = deflateSync(Buffer.from(content, 'latin1'));
  const objects = [
    Buffer.from('<< /Type /Catalog /Pages 2 0 R >>'),
    Buffer.from('<< /Type /Pages /Kids [3 0 R] /Count 1 >>'),
    Buffer.from('<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>'),
    Buffer.concat([Buffer.from(`<< /Length ${stream.length} /Filter /FlateDecode >>\nstream\n`), stream, Buffer.from('\nendstream')]),
    Buffer.from('<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>'),
  ];
  const parts = [Buffer.from('%PDF-1.7\n')];
  const offsets = [];
  for (const [index, body] of objects.entries()) {
    offsets.push(Buffer.concat(parts).length);
    parts.push(Buffer.from(`${index + 1} 0 obj\n`), body, Buffer.from('\nendobj\n'));
  }
  const startxref = Buffer.concat(parts).length;
  let trailer = `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`;
  for (const offset of offsets) trailer += `${String(offset).padStart(10, '0')} 00000 n \n`;
  trailer += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${startxref}\n%%EOF\n`;
  parts.push(Buffer.from(trailer));
  return Uint8Array.from(Buffer.concat(parts));
}

/** What `attachment_preview_source` answers: `[u32 LE length][JSON {kind}][bytes]`. */
export function sourceFrame(kind, bytes) {
  const json = Buffer.from(JSON.stringify({ kind }));
  const frame = Buffer.alloc(4 + json.length + bytes.length);
  frame.writeUInt32LE(json.length, 0);
  json.copy(frame, 4);
  frame.set(bytes, 4 + json.length);
  return [...frame];
}
