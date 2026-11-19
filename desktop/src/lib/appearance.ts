import { readPreference, savePreference } from './preferences';

export type Appearance = 'light' | 'dark' | 'system';

export function appearancePreference(): Appearance {
  const saved = readPreference('appearance');
  return saved === 'light' || saved === 'dark' || saved === 'system' ? saved : 'system';
}

export function systemPrefersDark(): boolean {
  return window.matchMedia('(prefers-color-scheme: dark)').matches;
}

export function resolvedAppearance(appearance: Appearance, systemDark: boolean): 'light' | 'dark' {
  return appearance === 'system' ? (systemDark ? 'dark' : 'light') : appearance;
}

export function saveAppearance(appearance: Appearance): void {
  savePreference('appearance', appearance);
}
