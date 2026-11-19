export function readPreference(key: string): string | null {
  try {
    // Preferences saved before the rebrandings used the `qemille:` or `sparklone:` prefix.
    return localStorage.getItem(`qamille:${key}`) ?? localStorage.getItem(`qemille:${key}`) ?? localStorage.getItem(`sparklone:${key}`);
  } catch {
    return null;
  }
}

export function savePreference(key: string, value: string): void {
  try {
    localStorage.setItem(`qamille:${key}`, value);
  } catch {
    // Navigation remains available if browser storage is disabled.
  }
}
