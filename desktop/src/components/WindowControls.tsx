import { useEffect, useRef, useState } from 'react';
import Button from '../ui/Button';
import Icon from '../ui/Icon';
import { errorText } from '../lib/api';
import { tr } from '../lib/i18n';
import { reportSnapButton } from '../lib/window-shell';
import type { Window as AppWindow } from '@tauri-apps/api/window';
import './WindowControls.css';

export type WindowControlsProps = { onerror: (message: string) => void };

let windowReady: Promise<AppWindow> | null = null;
const currentWindow = () =>
  (windowReady ??= import('@tauri-apps/api/window').then(({ getCurrentWindow }) =>
    getCurrentWindow(),
  ));

export default function WindowControls({ onerror }: WindowControlsProps) {
  const [maximized, setMaximized] = useState(false);
  // Windows handles the mouse over the maximize button (snap layouts, F7): it tells us hover and press.
  const [maximizeState, setMaximizeState] = useState<'rest' | 'hover' | 'pressed'>('rest');
  const maximizeButton = useRef<HTMLButtonElement>(null);
  // The listener outlives the render that made it: it reports through the latest callback.
  const report = useRef(onerror);
  report.current = onerror;
  const maximizeLabel = maximized
    ? tr('Ripristina finestra', 'Restore window')
    : tr('Ingrandisci finestra', 'Maximize window');

  async function updateMaximized() {
    try {
      setMaximized(await (await currentWindow()).isMaximized());
    } catch (error) {
      report.current(errorText(error));
    }
  }

  async function act(action: 'minimize' | 'toggleMaximize' | 'close') {
    try {
      // close() goes through the existing draft, settings and meeting close guards.
      await (await currentWindow())[action]();
    } catch (error) {
      report.current(errorText(error));
    }
  }

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void updateMaximized();
    void currentWindow()
      .then((appWindow) => appWindow.onResized(() => void updateMaximized()))
      .then((stop) => {
        if (disposed) stop();
        else unlisten = stop;
      })
      .catch((error) => report.current(errorText(error)));
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    const button = maximizeButton.current;
    const report = () => reportSnapButton(button);
    report();
    const observer = typeof ResizeObserver === 'function' ? new ResizeObserver(report) : null;
    if (button) observer?.observe(button);
    window.addEventListener('resize', report);
    let stop: (() => void) | undefined;
    let disposed = false;
    void import('@tauri-apps/api/event')
      .then(({ listen }) =>
        listen<'rest' | 'hover' | 'pressed'>('window-maximize-button', (event) => setMaximizeState(event.payload)),
      )
      .then((unlisten) => (disposed ? unlisten() : (stop = unlisten)))
      .catch(() => {});
    return () => {
      disposed = true;
      observer?.disconnect();
      window.removeEventListener('resize', report);
      stop?.();
      reportSnapButton(null);
    };
  }, []);

  return (
    <div
      className="window-controls"
      role="group"
      aria-label={tr('Controlli finestra', 'Window controls')}
    >
      <Button
        variant="icon"
        type="button"
        aria-label={tr('Riduci a icona', 'Minimize window')}
        tip={tr('Riduci a icona', 'Minimize window')}
        onClick={() => void act('minimize')}
      >
        <Icon name="minimize" size={16} />
      </Button>
      <Button
        variant="icon"
        type="button"
        aria-label={maximizeLabel}
        tip={maximizeLabel}
        ref={maximizeButton}
        data-caption={maximizeState === 'rest' ? undefined : maximizeState}
        onClick={() => void act('toggleMaximize')}
      >
        <Icon name={maximized ? 'window-restore' : 'window-maximize'} size={16} />
      </Button>
      <Button
        variant="icon"
        type="button"
        className="window-close"
        aria-label={tr('Chiudi finestra', 'Close window')}
        tip={tr('Chiudi finestra', 'Close window')}
        onClick={() => void act('close')}
      >
        <Icon name="close" size={18} />
      </Button>
    </div>
  );
}
