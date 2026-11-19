// Runs one embedded ArtCraft engine away from the UI thread. The WebAssembly module has no access
// to files, network or the desktop bridge: it only sees the bytes this worker hands it.
import type { PreviewKind, WorkerReply, WorkerRequest } from './protocol';

interface Engine {
  metadata(): string;
  renderPage(index: number, scale: number): Uint8Array;
  free(): void;
}
type Loader = () => Promise<(bytes: Uint8Array) => Engine>;

async function instantiate(
  init: (input: { module_or_path: ArrayBuffer }) => Promise<unknown>,
  wasmUrl: string,
) {
  const response = await fetch(wasmUrl);
  if (!response.ok) throw new Error(`Preview engine unavailable (${response.status}).`);
  await init({ module_or_path: await response.arrayBuffer() });
}

const loaders: Record<PreviewKind, Loader> = {
  pdf: async () => {
    const [engine, wasm] = await Promise.all([
      import('../../vendor/artcraft/pdfcraft/pdfcraft.js'),
      import('../../vendor/artcraft/pdfcraft/pdfcraft_bg.wasm?url'),
    ]);
    await instantiate(engine.default, wasm.default);
    return (bytes) => new engine.PdfDocument(bytes, null);
  },
  docx: async () => {
    const [engine, wasm] = await Promise.all([
      import('../../vendor/artcraft/wordcraft/wordcraft.js'),
      import('../../vendor/artcraft/wordcraft/wordcraft_bg.wasm?url'),
    ]);
    await instantiate(engine.default, wasm.default);
    return (bytes) => new engine.DocxDocument(bytes);
  },
  xlsx: async () => {
    const [engine, wasm] = await Promise.all([
      import('../../vendor/artcraft/gridcraft/gridcraft.js'),
      import('../../vendor/artcraft/gridcraft/gridcraft_bg.wasm?url'),
    ]);
    await instantiate(engine.default, wasm.default);
    return (bytes) => new engine.XlsxDocument(bytes);
  },
  pptx: async () => {
    const [engine, wasm] = await Promise.all([
      import('../../vendor/artcraft/deckcraft/deckcraft.js'),
      import('../../vendor/artcraft/deckcraft/deckcraft_bg.wasm?url'),
    ]);
    await instantiate(engine.default, wasm.default);
    return (bytes) => new engine.PptxDocument(bytes);
  },
};

const scope = self as unknown as {
  onmessage: ((event: MessageEvent<WorkerRequest>) => void) | null;
  postMessage(message: WorkerReply, transfer?: Transferable[]): void;
};
const loaded = new Map<PreviewKind, Promise<(bytes: Uint8Array) => Engine>>();
let document: Engine | null = null;

function load(kind: PreviewKind) {
  const known = loaders[kind];
  if (!known) throw new Error('No preview engine for this file type.');
  let engine = loaded.get(kind);
  if (!engine) {
    engine = known();
    loaded.set(kind, engine);
    engine.catch(() => loaded.delete(kind));
  }
  return engine;
}
function release() {
  document?.free();
  document = null;
}

scope.onmessage = async (event) => {
  const request = event.data;
  try {
    if (request.op === 'open') {
      const create = await load(request.kind);
      release();
      document = create(new Uint8Array(request.bytes));
      const metadata = JSON.parse(document.metadata()) as {
        pages: unknown[];
        warnings?: string[];
      };
      scope.postMessage({
        id: request.id,
        ok: true,
        pageCount: metadata.pages.length,
        warnings: metadata.warnings ?? [],
      });
    } else if (request.op === 'page') {
      if (!document) throw new Error('No document is open.');
      const png = document.renderPage(request.page, request.scale);
      const image = png.buffer.slice(png.byteOffset, png.byteOffset + png.byteLength) as ArrayBuffer;
      scope.postMessage({ id: request.id, ok: true, image }, [image]);
    } else {
      release();
      scope.postMessage({ id: request.id, ok: true });
    }
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    scope.postMessage({ id: request.id, ok: false, error: message });
  }
};
