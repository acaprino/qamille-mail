import { invoke, isTauri } from '@tauri-apps/api/core';
import { backendText } from './i18n';
export const desktop = isTauri();
export class CommandError extends Error {
  constructor(message: string, readonly code?: string) {
    super(message);
    this.name = 'CommandError';
  }
}
export async function call<T>(command: string, args: Record<string, unknown> = {}): Promise<T> {
  try {
    if (desktop) return await invoke<T>(command, args);
    const { previewCommand } = await import('./preview');
    return (await previewCommand(command, args)) as T;
  } catch (e) {
    if (e && typeof e === 'object' && 'message' in e)
      throw new CommandError(String(e.message), 'code' in e ? String(e.code) : undefined);
    throw new Error(String(e));
  }
}
export function errorText(error: unknown): string {
  return backendText(error instanceof Error ? error.message : String(error));
}
export function initials(name: string): string {
  return name
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((s) => s[0])
    .join('')
    .toUpperCase();
}
export { shortDate, fullDate } from './regional';
