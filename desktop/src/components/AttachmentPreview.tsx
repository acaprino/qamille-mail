import { useEffect, useRef } from 'react';
import Button from '../ui/Button';
import Icon from '../ui/Icon';
import Select from '../ui/Select';
import { CommandError, call, errorText } from '../lib/api';
import { decodeSourceFrame } from '../lib/embedded-preview/source-frame';
import { EmbeddedPreviewSession } from '../lib/embedded-preview/session';
import { humanSize } from '../lib/file-size';
import { fixedBackendText, isItalian, tr } from '../lib/i18n';
import { savedFileActions } from '../lib/saved-file';
import { useReactive } from '../lib/store';
import { cn } from '../lib/utils';
import type { NoticeAction } from '../ui/Toaster';
import type { Attachment } from '../lib/types';
import './AttachmentPreview.css';

export type AttachmentPreviewProps = {
  emailId: number;
  attachment: Attachment;
  /** A document inside an attached message: attachment indexes from `attachment`. */
  path?: number[];
  /** Rendered inside the attachment panel, which owns the title and close button. */
  embedded?: boolean;
  notify: (message: string, error?: boolean, actions?: NoticeAction[]) => void;
  close: () => void;
};

const zooms = [
  { label: '75%', dpi: 72 },
  { label: '100%', dpi: 96 },
  { label: '150%', dpi: 144 },
  { label: '200%', dpi: 192 },
];
const noPath: number[] = [];

function uniqueWarnings(values: string[]): string[] {
  return [...new Set(values.map((value) => value.trim()).filter(Boolean))];
}

export default function AttachmentPreview({
  emailId,
  attachment,
  path = noPath,
  embedded = false,
  notify,
  close,
}: AttachmentPreviewProps) {
  // Everything here is read again after awaits: it lives in one always-current object.
  const s = useReactive(() => ({
    status: 'loading' as 'loading' | 'visual' | 'text' | 'error',
    session: null as EmbeddedPreviewSession | null,
    page: 1,
    pageCount: 0,
    zoomIndex: 1,
    imageUrl: '',
    pageLoading: false,
    pageError: '',
    text: '',
    textLoading: false,
    errorMessage: '',
    warnings: [] as string[],
    saving: false,
  }));
  const tokens = useRef({ open: 0, page: 0, text: 0 });
  // The latest props, for work that started before they changed.
  const props = useRef({ emailId, attachment, path, notify, close });
  props.current = { emailId, attachment, path, notify, close };
  const nested = () => (props.current.path.length ? { path: [...props.current.path] } : {});

  function releaseImage() {
    if (s.imageUrl) URL.revokeObjectURL(s.imageUrl);
    s.imageUrl = '';
  }
  function releaseDocument() {
    const current = s.session;
    s.session = null;
    releaseImage();
    current?.dispose();
  }

  async function open() {
    const mine = ++tokens.current.open;
    tokens.current.page += 1;
    tokens.current.text += 1;
    releaseDocument();
    s.page = 1;
    s.pageCount = 0;
    s.text = '';
    s.textLoading = false;
    s.pageLoading = false;
    s.pageError = '';
    s.errorMessage = '';
    s.warnings = [];
    s.status = 'loading';
    let next: EmbeddedPreviewSession | null = null;
    try {
      const source = decodeSourceFrame(
        await call<ArrayBuffer>('attachment_preview_source', {
          emailId: props.current.emailId,
          attachmentId: props.current.attachment.id,
          ...nested(),
        }),
      );
      if (mine !== tokens.current.open) return;
      next = new EmbeddedPreviewSession();
      await next.open(source.kind, source.bytes);
      if (mine !== tokens.current.open) {
        next.dispose();
        return;
      }
      s.session = next;
      s.pageCount = next.pageCount;
      s.warnings = uniqueWarnings(next.warnings);
      s.status = 'visual';
      await loadPage(mine);
    } catch (error) {
      next?.dispose();
      if (mine !== tokens.current.open) return;
      const code = error instanceof CommandError ? error.code : undefined;
      if (code === 'preview_unsupported') {
        await loadText(mine);
        return;
      }
      s.status = 'error';
      s.errorMessage = errorText(error);
    }
  }

  async function loadPage(expectedOpen = tokens.current.open) {
    const current = s.session;
    if (!current) return;
    const mine = ++tokens.current.page;
    s.pageLoading = true;
    s.pageError = '';
    try {
      const image = await current.render(s.page, zooms[s.zoomIndex].dpi);
      if (mine !== tokens.current.page || expectedOpen !== tokens.current.open) return;
      releaseImage();
      s.imageUrl = URL.createObjectURL(new Blob([image], { type: 'image/png' }));
    } catch (error) {
      if (mine !== tokens.current.page || expectedOpen !== tokens.current.open) return;
      s.pageError = errorText(error);
    } finally {
      if (mine === tokens.current.page && expectedOpen === tokens.current.open) s.pageLoading = false;
    }
  }

  async function loadText(expectedOpen = tokens.current.open) {
    const mine = ++tokens.current.text;
    s.textLoading = true;
    s.status = 'loading';
    try {
      const result = await call<string>('read_attachment', {
        emailId: props.current.emailId,
        attachmentId: props.current.attachment.id,
        ...nested(),
      });
      if (mine !== tokens.current.text || expectedOpen !== tokens.current.open) return;
      s.text = result;
      s.errorMessage = '';
      s.status = 'text';
    } catch (error) {
      if (mine !== tokens.current.text || expectedOpen !== tokens.current.open) return;
      const message = errorText(error);
      if (s.session) {
        s.status = 'visual';
        s.errorMessage = message;
        props.current.notify(message, true);
      } else {
        s.status = 'error';
        s.errorMessage = message;
      }
    } finally {
      if (mine === tokens.current.text && expectedOpen === tokens.current.open) s.textLoading = false;
    }
  }

  const pathKey = path.join('/');
  useEffect(() => {
    void open();
    // open reads the current props itself.
  }, [emailId, attachment.id, pathKey]);
  useEffect(
    () => () => {
      tokens.current.open += 1;
      tokens.current.page += 1;
      tokens.current.text += 1;
      releaseDocument();
    },
    [],
  );

  async function saveAttachment() {
    if (s.saving) return;
    s.saving = true;
    try {
      const saved = await call<{ path: string; token: number } | null>('attachment_save', {
        emailId: props.current.emailId,
        attachmentId: props.current.attachment.id,
        italian: isItalian(),
        ...nested(),
      });
      if (saved) {
        const { notify } = props.current;
        notify(
          tr(`Allegato salvato: ${saved.path}`, `Attachment saved: ${saved.path}`),
          false,
          savedFileActions(saved.token, notify),
        );
      }
    } catch (error) {
      props.current.notify(errorText(error), true);
    } finally {
      s.saving = false;
    }
  }
  function showVisual() {
    if (!s.session) return;
    s.status = 'visual';
    s.errorMessage = '';
    if (!s.imageUrl) void loadPage();
  }
  function changePage(delta: number) {
    const next = s.page + delta;
    if (next < 1 || next > s.pageCount || s.pageLoading) return;
    s.page = next;
    void loadPage();
  }
  function changeZoom() {
    if (!s.session) return;
    void loadPage();
  }
  function handleClose() {
    tokens.current.open += 1;
    tokens.current.page += 1;
    tokens.current.text += 1;
    releaseDocument();
    close();
  }

  const { status, session, page, pageCount, zoomIndex, imageUrl, pageLoading, pageError, text, textLoading } = s;

  return (
    <div
      className={cn('attachment-preview', embedded && 'embedded')}
      role="region"
      aria-label={tr('Anteprima dell’allegato', 'Attachment preview')}
    >
      <div className="preview-header">
        <div className={cn('preview-title', embedded && 'visually-hidden')}>
          <strong title={attachment.name}>{attachment.name}</strong>{' '}
          <small>
            {humanSize(attachment.size)} ·{' '}
            {status === 'text'
              ? tr('Testo estratto', 'Extracted text')
              : status === 'visual'
                ? tr('Anteprima documento', 'Document preview')
                : tr('Anteprima', 'Preview')}
          </small>
        </div>{' '}
        <div className="preview-actions">
          <Button
            variant="icon"
            tip={tr('Scarica l’allegato', 'Download attachment')}
            aria-label={tr('Scarica l’allegato', 'Download attachment')}
            disabled={s.saving}
            onClick={() => void saveAttachment()}
          >
            <Icon name="download" size={16} />
          </Button>{' '}
          {session && (
            <>
              <Button
                variant="text"
                aria-pressed={status === 'visual'}
                disabled={status === 'loading'}
                onClick={showVisual}
              >
                {tr('Pagine', 'Pages')}
              </Button>{' '}
              <Button
                variant="text"
                aria-pressed={status === 'text'}
                disabled={textLoading || status === 'loading'}
                onClick={() => void loadText()}
              >
                {tr('Testo', 'Text')}
              </Button>{' '}
            </>
          )}
          {!embedded && (
            <Button
              variant="icon"
              aria-label={tr('Chiudi l’anteprima dell’allegato', 'Close attachment preview')}
              tip={tr('Chiudi l’anteprima', 'Close preview')}
              onClick={handleClose}
            >
              <Icon name="close" size={16} />
            </Button>
          )}
        </div>
      </div>

      {s.errorMessage && (
        <div className="preview-alert" role="alert">
          <p>{s.errorMessage}</p>
          {!session && (
            <Button variant="text" disabled={textLoading} onClick={() => void loadText()}>
              {tr('Mostra il testo estratto', 'Show extracted text')}
            </Button>
          )}
        </div>
      )}
      {!!s.warnings.length && (
        <div className="preview-note" role="status">
          {s.warnings.map((warning) => (
            <p key={warning}>{fixedBackendText(warning)}</p>
          ))}
        </div>
      )}

      {status === 'loading' ? (
        <p className="preview-status" role="status">
          {textLoading ? tr('Estrazione del testo…', 'Extracting text…') : tr('Preparazione dell’anteprima…', 'Preparing preview…')}
        </p>
      ) : status === 'visual' ? (
        <>
          <div className="preview-toolbar">
            <Button
              variant="icon"
              aria-label={tr('Pagina precedente', 'Previous page')}
              tip={tr('Pagina precedente', 'Previous page')}
              disabled={page <= 1 || pageLoading}
              onClick={() => changePage(-1)}
            >
              <span className="chevron-left">
                <Icon name="chevron" size={16} />
              </span>
            </Button>{' '}
            <span className="preview-position" aria-live="polite">
              {tr(`Pagina ${page} di ${pageCount}`, `Page ${page} of ${pageCount}`)}
            </span>{' '}
            <Button
              variant="icon"
              aria-label={tr('Pagina successiva', 'Next page')}
              tip={tr('Pagina successiva', 'Next page')}
              disabled={page >= pageCount || pageLoading}
              onClick={() => changePage(1)}
            >
              <Icon name="chevron" size={16} />
            </Button>{' '}
            <Select
              aria-label={tr('Zoom', 'Zoom')}
              value={String(zoomIndex)}
              items={zooms.map((zoom, index) => ({ value: String(index), label: zoom.label }))}
              disabled={pageLoading}
              onChange={(next) => {
                s.zoomIndex = Number(next);
                changeZoom();
              }}
            />{' '}
            <Button
              variant="icon"
              tip={tr('Ricarica la pagina', 'Reload page')}
              aria-label={tr('Ricarica la pagina', 'Reload page')}
              disabled={pageLoading}
              onClick={() => void loadPage()}
            >
              <Icon name="refresh" size={16} />
            </Button>
          </div>
          <div className="preview-canvas" aria-busy={pageLoading}>
            {pageLoading && (
              <p className="preview-status floating" role="status">
                {tr('Caricamento della pagina…', 'Loading page…')}
              </p>
            )}
            {pageError && (
              <p className="preview-alert floating" role="alert">
                {pageError}
              </p>
            )}
            {imageUrl && (
              <img
                src={imageUrl}
                alt={tr(
                  `Pagina ${page} dell’allegato ${attachment.name}`,
                  `Page ${page} of attachment ${attachment.name}`,
                )}
              />
            )}
          </div>
        </>
      ) : status === 'text' ? (
        <pre>{fixedBackendText(text)}</pre>
      ) : null}
    </div>
  );
}
