// WindowControls alone, in a 40 px bar like the topbar. The test supplies the Tauri window mock.
import { useState } from 'react';
import WindowControls from '../../../src/components/WindowControls';

export default function NoticesGallery() {
  const [errors, setErrors] = useState<string[]>([]);
  return (
    <>
      <header aria-label="Bar" style={{ display: 'flex', height: 40, background: 'var(--surface)' }}>
        <WindowControls onerror={(message) => setErrors((all) => [...all, message])} />
      </header>
      <output aria-label="Window errors">{errors.join('|')}</output>
    </>
  );
}
