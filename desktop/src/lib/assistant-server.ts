// The assistant «Server Qamille» (gateway LLM, wave 5). Pure functions, without the interface
// language so they can be tested alone: texts are `{ it, en }` pairs the components pass to
// `tr`. The consent text names the provider the server declares in `GET /v1/info` (INV-34);
// the provider list puts the server first and the keys of the person under «Use your own key».
import type { LlmInfo } from './server-connect';

export type Text = { it: string; en: string };

/** `server_llm_status`: where the assistant of the connected server stands. */
export type LlmStatus =
  | { kind: 'unavailable' }
  | { kind: 'ready'; label: string; external: boolean }
  | { kind: 'consent_needed'; label: string; external: boolean };

/** The sentence of «Before you connect» about the assistant; null when the server has none. */
export function consentText(llm: LlmInfo | null | undefined): Text | null {
  if (!llm) return null;
  return llm.external
    ? {
        it: `Le richieste all'assistente vengono inoltrate a ${llm.label}, un servizio esterno a questo server.`,
        en: `Assistant requests are forwarded to ${llm.label}, a service outside this server.`,
      }
    : { it: "L'assistente usa un modello che gira su questo server.", en: 'The assistant uses a model that runs on this server.' };
}

export type ProviderItem = { value: string; label: Text; ownKey: boolean };

/** «Server Qamille» first, when the server has an assistant; the others are «your own key». */
export function providerItems(status: LlmStatus | null): ProviderItem[] {
  const server = status !== null && status.kind !== 'unavailable';
  return [
    ...(server ? [{ value: 'QamilleServer', label: { it: 'Server Qamille', en: 'Qamille server' }, ownKey: false }] : []),
    { value: 'Offline', label: { it: 'Regole integrate · offline', en: 'Built-in rules · offline' }, ownKey: false },
    { value: 'Anthropic', label: { it: 'Anthropic', en: 'Anthropic' }, ownKey: true },
    { value: 'OpenAiCompatible', label: { it: 'Compatibile con OpenAI', en: 'OpenAI-compatible' }, ownKey: true },
    { value: 'Ollama', label: { it: 'Ollama · locale', en: 'Ollama · local' }, ownKey: true },
  ];
}

/** The server changed its provider since the person agreed: the assistant waits for a new yes. */
export function needsReconsent(status: LlmStatus | null): boolean {
  return status?.kind === 'consent_needed';
}

/** The warning above the provider list, or null. */
export function reconsentText(status: LlmStatus | null): Text | null {
  if (!status || status.kind !== 'consent_needed') return null;
  const external = status.external;
  return {
    it: `Il server ora usa un altro provider (${status.label}${external ? ', servizio esterno' : ''}): conferma per continuare.`,
    en: `The server now uses another provider (${status.label}${external ? ', an external service' : ''}): confirm to continue.`,
  };
}

/** A friendly name for the `llm-*` models of `GET /health`; null for any other model. */
export function llmModelName(id: string): Text | null {
  const size = /^llm-(piccolo|medio|grande)$/.exec(id)?.[1];
  if (!size) return id.startsWith('llm-') ? { it: id, en: id } : null;
  const english = size === 'piccolo' ? 'small' : size === 'medio' ? 'medium' : 'large';
  return { it: `Assistente ${size}`, en: `Assistant ${english}` };
}
