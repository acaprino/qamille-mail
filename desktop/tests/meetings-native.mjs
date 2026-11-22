// Native integration with an owned sample workspace and silent generated audio.
// Never calls meeting_start, remote transcription or model installation.
import { chromium, expect } from '@playwright/test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';

const root = path.resolve(import.meta.dirname, '../..');
const data = path.join(root, 'checks/meetings/test-data', `native-${Date.now()}`);
const folder = path.join(data, 'meetings/9001');
await fs.mkdir(folder, { recursive: true });
// The X closes Qamille here, as before the tray (src-tauri/src/background.rs).
await fs.writeFile(path.join(data, 'background.json'), JSON.stringify({ close_to_tray: false, tray_notice_shown: true }));
const sample = JSON.parse(await fs.readFile(path.join(root, 'desktop/src/lib/sample.json'), 'utf8'));
const meeting = { ...sample.meetings[0], id: 9001, title: 'Meeting sintetico nativo', date: Math.floor(Date.now() / 1000), duration_secs: 2, event_id: null, audio_path: null, notes: 'Originale sintetico', transcript: '', summary: '', key_points: [], action_items: [] };
const capture = { meeting_id: 9001, state: 'saved', requires_receipts: true, elapsed_ms: 2000, saved_ms: 2000, microphone_level: 0, system_level: 0, error: null, chunks: [{ source: 'microphone', sequence: 0, path: 'synthetic.wav', start_ms: 0, end_ms: 2000, sample_rate: 16000, channels: 1 }] };
const detail = { schema_version: 2, revision: 0, meeting, request_id: 'synthetic-native', capture, transcripts: [], active_transcript: null, notes: [{ id: 'free', text: meeting.notes, at_ms: null, revision: 1 }], note_history: [], summaries: [], active_summary: null, jobs: [], included_in_search: true, deleted: false };
// Finished output was synced, but the process died before updating the session.
const jobId = 'synthetic-result-ready-before-crash';
detail.jobs.push({ id: jobId, kind: 'transcribe', state: 'running', input_revision: 0, progress: 99, error: null, created_at: meeting.date, options: null, language: null });
const checkpoint = { schema_version: 1, meeting_id: 9001, job_id: jobId, input_revision: 0, created_at: meeting.date, transcript_version: null, cancelled: false, error: null, output: { Transcript: { id: '', created_at: 0, model: 'synthetic-completed-output', language: 'it', complete: true, segments: [{ id: 'native-recovered-result', start_ms: 0, end_ms: 2000, text: 'Trascrizione sintetica recuperata dal crash.', original_text: 'Trascrizione sintetica recuperata dal crash.', source: 'import', confidence: null }] } } };
const payload = JSON.stringify(checkpoint);
const hash = value => createHash('sha256').update(value).digest('hex');
await fs.mkdir(path.join(folder, 'job-results'), { recursive: true });
await fs.writeFile(path.join(folder, 'job-results', `${hash(jobId)}.json.writing`), JSON.stringify({ payload, sha256: hash(payload) }));
const wave = Buffer.alloc(44 + 32000 * 2);
wave.write('RIFF', 0); wave.writeUInt32LE(wave.length - 8, 4); wave.write('WAVEfmt ', 8);
wave.writeUInt32LE(16, 16); wave.writeUInt16LE(1, 20); wave.writeUInt16LE(1, 22);
wave.writeUInt32LE(16000, 24); wave.writeUInt32LE(32000, 28); wave.writeUInt16LE(2, 32); wave.writeUInt16LE(16, 34);
wave.write('data', 36); wave.writeUInt32LE(64000, 40);
await fs.writeFile(path.join(folder, 'synthetic.wav'), wave);
await fs.writeFile(path.join(folder, 'session.json'), JSON.stringify(detail));
await fs.writeFile(path.join(folder, 'capture.json'), JSON.stringify(capture));
// The compiled app must recover valid audio despite both primary indexes being broken.
await fs.writeFile(path.join(folder, 'session.json.prev'), JSON.stringify(detail));
await fs.writeFile(path.join(folder, 'session.json'), '{interrupted session write');
await fs.writeFile(path.join(folder, 'capture.json'), '{interrupted capture write');
const port = 40000 + Math.floor(Math.random() * 10000);
const processHandle = spawn(path.join(root, 'app/target/debug/qamille-desktop.exe'), [], {
  cwd: root, windowsHide: true, stdio: 'pipe',
  env: { ...process.env, QAMILLE_DATA: data, QAMILLE_DEMO: '1', WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`, WEBVIEW2_USER_DATA_FOLDER: path.join(data, 'webview') },
});
let output = '';
processHandle.stderr.on('data', (bytes) => { output += bytes; });
processHandle.stdout.on('data', (bytes) => { output += bytes; });
let browser, page;
const errors = [];
try {
  for (let attempt = 0; attempt < 80; attempt++) {
    if (processHandle.exitCode !== null) throw new Error(`Test instance exited (another app may already be open): ${output}`);
    try { if ((await fetch(`http://127.0.0.1:${port}/json/version`)).ok) break; } catch { /* Startup only. */ }
    if (attempt === 79) throw new Error(`Native test window unavailable: ${output}`);
    await new Promise(resolve => setTimeout(resolve, 250));
  }
  browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
  page = browser.contexts()[0].pages()[0];
  page.setDefaultTimeout(15000);
  page.on('pageerror', error => errors.push(error.message));
  await page.locator('.compose-main').waitFor();
  const invoke = (command, args = {}) => page.evaluate(({ command, args }) => window.__TAURI_INTERNALS__.invoke(command, args), { command, args });
  const workspace = await invoke('workspace_snapshot');
  assert.ok(workspace.accounts.length && workspace.accounts.every(account => account.kind === 'Sample'), 'Only synthetic accounts are allowed');
  const archive = await invoke('meeting_list_page', { offset: 0, limit: 50 });
  assert.ok(archive.items.some(item => item.meeting.id === 9001));
  assert.ok(archive.items.every(item => !item.meeting.notes && !item.capture.chunks.length));
  assert.equal((await invoke('meeting_capture_status')).state, 'idle');
  const result = await invoke('meeting_read', { id: 9001 });
  assert.equal(result.jobs[0].state, 'completed');
  assert.equal(result.transcripts.length, 1);
  assert.equal(result.transcripts[0].model, 'synthetic-completed-output');
  assert.equal(result.meeting.transcript, 'Trascrizione sintetica recuperata dal crash.');
  assert.equal((await invoke('meeting_read', { id: 9001 })).transcripts.length, 1);
  assert.equal((await fs.readdir(path.join(folder, 'job-results'))).length, 0);
  console.log('PASS native startup restores a finished AI result exactly once without a provider request');
  const bytes = await invoke('meeting_audio_range', { id: 9001, startMs: 0, durationMs: 2000 });
  assert.equal(Buffer.from(bytes).subarray(0, 4).toString(), 'RIFF');
  await page.getByRole('button', { name: /^(Meetings?|Riunioni)$/ }).click();
  await page.getByRole('button', { name: /Meeting sintetico nativo/ }).click();
  await page.getByRole('button', { name: 'Recupera audio', exact: true }).click();
  await expect.poll(async () => (await invoke('meeting_read', { id: 9001 })).capture.state).toBe('saved');
  const recovered = await invoke('meeting_read', { id: 9001 });
  assert.equal(recovered.capture.incomplete, true);
  assert.equal(recovered.capture.chunks.length, 1);
  assert.deepEqual(recovered.capture.unverified_chunks, ['synthetic.wav']);
  assert.ok(recovered.recovery_notice);
  assert.equal((await fs.readFile(path.join(folder, 'synthetic.wav'))).equals(wave), true);
  console.log('PASS native startup recovers damaged indexes without changing saved audio');
  console.log('PASS native IPC and lightweight archive');

  // The page has no audio player and no notes field any more: only the composer draft is open at close.
  await page.locator('.compose-main').click();
  const bodyName = /^(Message body|Corpo del messaggio|Corpo messaggio)$/;
  const panel = page.getByRole('tabpanel').filter({ has: page.getByRole('textbox', { name: bodyName }) });
  await panel.getByRole('textbox', { name: /^(Subject|Oggetto)$/ }).fill('Bozza sintetica chiusura');
  await panel.getByRole('textbox', { name: bodyName }).fill('La chiusura conserva questa bozza.');
  const closed = page.waitForEvent('close');
  await invoke('plugin:window|close', { label: 'main' });
  await closed;
  const saved = JSON.parse(await fs.readFile(path.join(folder, 'session.json'), 'utf8'));
  assert.equal(saved.capture.incomplete, true);
  const store = JSON.parse(await fs.readFile(path.join(data, 'store.json'), 'utf8'));
  assert.ok(store.drafts.some(draft => draft.subject === 'Bozza sintetica chiusura' && draft.body.includes('La chiusura conserva questa bozza.')));
  assert.deepEqual(errors, []);
  console.log('PASS native window close flushes the composer draft and keeps the recovered meeting');
  await fs.writeFile(path.join(data, 'result.json'), JSON.stringify({ passed: 4, errors, recorded: false }, null, 2));
} catch (error) {
  await page?.screenshot({ path: path.join(data, 'failure.png') }).catch(() => {});
  throw error;
} finally {
  await browser?.close().catch(() => {});
  if (processHandle.exitCode === null) processHandle.kill();
}
