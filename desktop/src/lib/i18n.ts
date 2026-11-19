import { backendTextEnglish, backendTextItalian } from './backend-text';
import { readPreference, savePreference } from './preferences';
import { reactive, useStore } from './store';

// Interface language. It defaults to the regional format's language, and can be
// chosen separately on this device, so date formats and words stay independent.
export type LanguageChoice = 'auto' | 'it' | 'en';

function storedChoice(): LanguageChoice {
  const value = readPreference('ui-language');
  return value === 'it' || value === 'en' ? value : 'auto';
}

// The language state lives in a store: a component that shows translated words re-renders when it
// changes because the root (App) subscribes with `useLanguage()` and every child renders under it.
export const language = reactive({
  choice: storedChoice(),
  locale: typeof navigator !== 'undefined' ? navigator.language : 'en-US',
});

/** Re-renders the calling component when the interface language changes. */
export function useLanguage(): boolean {
  useStore(language);
  return isItalian();
}

export function languageChoice(): LanguageChoice {
  return language.choice;
}

export function setLanguageChoice(value: LanguageChoice): void {
  language.choice = value;
  savePreference('ui-language', value);
  syncDocumentLanguage();
}

export function setLanguageLocale(value: string): void {
  language.locale = value;
  syncDocumentLanguage();
}

export function isItalian(): boolean {
  return language.choice === 'auto'
    ? language.locale.toLowerCase().startsWith('it')
    : language.choice === 'it';
}

export function tr(it: string, en: string): string {
  return isItalian() ? it : en;
}

// Text written by the Rust backend, shown in the interface language.
export function backendText(message: string): string {
  return isItalian() ? backendTextItalian(message) : backendTextEnglish(message);
}

// For values that may be mail or document content: only a whole single-line value
// can be a fixed backend message, so longer content is never rewritten.
export function fixedBackendText(value: string): string {
  return value.includes('\n') ? value : backendText(value);
}

const FOLDERS: Record<string, [string, string]> = {
  Inbox: ['Posta in arrivo', 'Inbox'],
  Tasks: ['Da fare', 'To do'],
  Snoozed: ['Posticipati', 'Snoozed'],
  Drafts: ['Bozze', 'Drafts'],
  Sent: ['Inviati', 'Sent'],
  Archive: ['Archivio', 'Archive'],
  Trash: ['Cestino', 'Trash'],
  Spam: ['Spam', 'Spam'],
  Calendar: ['Calendario', 'Calendar'],
  Contacts: ['Rubrica', 'Contacts'],
  Meetings: ['Meeting', 'Meetings'],
  Assistant: ['Assistente', 'Assistant'],
  Activity: ['Attività', 'Activity'],
  Settings: ['Impostazioni', 'Settings'],
};
const SECTIONS: Record<string, [string, string]> = {
  Priority: ['Priorità', 'Priority'],
  New: ['Nuovi', 'New'],
  Notifications: ['Notifiche', 'Notifications'],
  Newsletters: ['Newsletter', 'Newsletters'],
  Pins: ['Fissati', 'Pins'],
  Seen: ['Già letti', 'Seen'],
};
const CATEGORIES: Record<string, [string, string]> = {
  Priority: ['Priorità', 'Priority'],
  Personal: ['Personale', 'Personal'],
  Notification: ['Notifica', 'Notification'],
  Newsletter: ['Newsletter', 'Newsletter'],
  Invitation: ['Invito', 'Invitation'],
};
function labelFrom(table: Record<string, [string, string]>, key: string): string {
  const entry = table[key];
  return entry ? tr(entry[0], entry[1]) : key;
}
export const folderLabel = (folder: string) =>
  folder.startsWith('label:') ? folder.slice(6) : labelFrom(FOLDERS, folder);
export const sectionLabel = (section: string) => labelFrom(SECTIONS, section);
export const categoryLabel = (category: string) => labelFrom(CATEGORIES, category);

function syncDocumentLanguage(): void {
  if (typeof document !== 'undefined') document.documentElement.lang = isItalian() ? 'it' : 'en';
}

syncDocumentLanguage();
