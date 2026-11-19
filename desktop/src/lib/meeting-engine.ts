// The two transcription engines of Settings > Meetings (spec D7). Settings and jobs saved before
// the compute server said `local` (whisper.cpp in the app, now in the compute server) and
// `remote` (the OpenAI-compatible endpoint): they read as the new names.
import type { Settings } from './types';

export type MeetingEngine = Settings['meeting_engine'];

export function meetingEngine(value: string | null | undefined): MeetingEngine {
  if (value === 'external' || value === 'remote') return 'external';
  return 'compute';
}
