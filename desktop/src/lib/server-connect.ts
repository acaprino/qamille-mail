// The guided form that connects this computer to a Qamille server (access wave 2, spec A8):
// «Server» → «Before you connect» → «Sign in» → («New password» | device list) → done. Pure: the
// component owns the state and makes the calls; these functions only say where the form goes.
// The Professional build starts from consent with its preset address and has no «Server» step.

export type Step = 'server' | 'consent' | 'sign_in' | 'new_password' | 'device_limit' | 'done';

export interface PlanInfo {
  id: string;
  name: string;
  /** `month` or `year`. */
  interval: string;
  price_cents: number;
  currency: string;
  /** What the plan includes each month, when the server limits compute (wave 9). */
  quotas?: QuotaInfo | null;
}
/** The monthly limits of a plan: minutes of audio, tokens of semantic search, tokens of the assistant. */
export interface QuotaInfo {
  audio_minutes: number;
  embedded_tokens: number;
  llm_tokens: number;
}
/** One meter of the month. */
export interface Meter {
  used: number;
  limit: number;
}
/** `GET /v1/account` → `subscription.usage`: calendar month in UTC, back to zero on `renews_at`. */
export interface UsageInfo {
  period_start: string;
  renews_at: string;
  audio_minutes: Meter;
  embedded_tokens: Meter;
  llm_tokens: Meter;
}
/** The documents a new account accepts (wave 9): addresses to open and the version to send back. */
export interface LegalInfo {
  terms_url?: string | null;
  privacy_url?: string | null;
  terms_version?: string | null;
}
/** A server that sells subscriptions (spec «Vendita Professional»). */
export interface BillingInfo {
  provider: string;
  /** A fake payment processor: no money moves. */
  test_mode: boolean;
  currency: string;
  trial_days: number;
  plans: PlanInfo[];
}
/** What `GET /v1/info` declares of the assistant: never a key or an address. */
export interface LlmInfo {
  model: string;
  models: string[];
  /** The server forwards to a service outside its own network. */
  external: boolean;
  label: string;
  context_tokens: number | null;
}
export interface ServerInfo {
  version: string;
  api: number;
  signup: boolean;
  password_reset: boolean;
  edition: string;
  /** Browser sign-ins the server offers (`google`). Absent on older servers. */
  login_providers?: string[];
  /** A Google sign-in may create an account. */
  google_signup?: boolean;
  billing?: BillingInfo | null;
  /** The assistant of the server (gateway LLM). Absent on servers without one. */
  llm?: LlmInfo | null;
  /** Mailbox synchronization (wave 7). Absent on servers without it. */
  sync?: { available: boolean } | null;
  /** Terms and privacy notice of the server (wave 9). Absent on servers that do not ask for them. */
  legal?: LegalInfo | null;
}
export interface Subscription {
  /** `none`, `trialing`, `active`, `past_due`, `canceled` or `expired`. */
  status: string;
  entitled: boolean;
  plan_id: string | null;
  plan_name: string | null;
  provider: string | null;
  current_period_end: string | null;
  cancel_at_period_end: boolean;
  manageable: boolean;
  /** The consumption of the month, on servers that limit compute (wave 9). */
  usage?: UsageInfo | null;
}
/** `server_account_info`: never a key. */
export interface AccountInfo {
  user_id: string;
  username: string;
  email: string | null;
  has_password: boolean;
  identities: string[];
  billing_enabled: boolean;
  subscription: Subscription | null;
}
export interface DeviceInfo {
  id: string;
  name: string;
  created_at: string;
  last_seen_at: string | null;
  current: boolean;
}
export type Probe =
  | { kind: 'found'; info: ServerInfo }
  | { kind: 'incompatible'; info: ServerInfo }
  | { kind: 'unreachable' }
  | { kind: 'not_qamille' }
  | { kind: 'https_required' }
  | { kind: 'invalid' };
export type ConnectOutcome =
  | { kind: 'connected'; status: unknown }
  | { kind: 'password_change_required' }
  | { kind: 'device_limit'; devices: DeviceInfo[] };

export interface ConnectForm {
  step: Step;
  /** The Professional address; null in the self-hosted build. */
  preset: string | null;
  /** The address as typed in the «Server» step. */
  url: string;
  /** The last probe answer, for the address it was asked for. */
  probe: { url: string; result: Probe | null } | null;
  /** Addresses the person already agreed to (`compute_consented_urls`). */
  consented: string[];
  identifier: string;
  /** Kept only while the form is open: cleared at `done` and at every cancel. */
  password: string;
  devices: DeviceInfo[];
}

/** The comparable form of an address: what the backend normalizes, for consent lookups. */
export function normalizeUrl(url: string): string {
  const trimmed = url.trim();
  const match = /^([a-zA-Z]+):\/\/([^/?#]+)\/?$/.exec(trimmed);
  return match ? `${match[1].toLowerCase()}://${match[2].toLowerCase()}` : trimmed;
}

/**
 * `reconnect`: «Sign in again» after the server stopped knowing this device (spec A8, EX-15):
 * the form opens on «Sign in» for the session's address (on consent, if it was withdrawn).
 */
export function start(options: { preset?: string | null; consented?: string[]; url?: string; reconnect?: string | null }): ConnectForm {
  const preset = options.preset ?? null;
  const consented = (options.consented ?? []).map(normalizeUrl);
  const base: ConnectForm = {
    step: 'server',
    preset,
    url: preset ?? options.url ?? '',
    probe: null,
    consented,
    identifier: '',
    password: '',
    devices: [],
  };
  const fixed = preset ?? options.reconnect ?? null;
  if (!fixed) return base;
  return { ...base, url: fixed, step: consented.includes(normalizeUrl(fixed)) ? 'sign_in' : 'consent' };
}

/** A new address typed: an answer for an older one no longer counts. */
export function typeUrl(form: ConnectForm, url: string): ConnectForm {
  if (form.preset) return form;
  return { ...form, url, probe: { url, result: null } };
}

/** A probe answered; an answer for an address that is no longer the typed one is dropped. */
export function probed(form: ConnectForm, url: string, result: Probe): ConnectForm {
  if (url !== form.url) return form;
  return { ...form, probe: { url, result } };
}

/** «Next» in the «Server» step: only with a Qamille server found at the typed address. */
export function canLeaveServer(form: ConnectForm): boolean {
  return form.step === 'server' && form.probe?.url === form.url && form.probe.result?.kind === 'found';
}

export function leaveServer(form: ConnectForm): ConnectForm {
  if (!canLeaveServer(form)) return form;
  return { ...form, step: form.consented.includes(normalizeUrl(form.url)) ? 'sign_in' : 'consent' };
}

/** The person agreed to send content to this address. */
export function consentGiven(form: ConnectForm): ConnectForm {
  if (form.step !== 'consent') return form;
  const url = normalizeUrl(form.url);
  return { ...form, step: 'sign_in', consented: form.consented.includes(url) ? form.consented : [...form.consented, url] };
}

/** Where a sign-in (or a new password) leads. The password goes once the form is done. */
export function signedIn(form: ConnectForm, outcome: ConnectOutcome): ConnectForm {
  switch (outcome.kind) {
    case 'connected':
      return { ...form, step: 'done', password: '', devices: [] };
    case 'password_change_required':
      return { ...form, step: 'new_password' };
    case 'device_limit':
      return { ...form, step: 'device_limit', devices: outcome.devices };
  }
}

/** One device was disabled: back to signing in with the same credentials. */
export function deviceDisabled(form: ConnectForm, id: string): ConnectForm {
  return { ...form, step: 'sign_in', devices: form.devices.filter((device) => device.id !== id) };
}

/** «Cancel»: nothing written, no password kept. */
export function cancelled(form: ConnectForm): ConnectForm {
  return { ...form, password: '', devices: [] };
}

/** The server of the Docker guide on this same PC (wave 2, E10). */
export const LOCAL_SERVER_URL = 'http://127.0.0.1:8787';

/**
 * True when the «Server» step is empty, there is no preset and the local probe found a Qamille
 * server: the step then offers «Server su questo PC (Docker)».
 */
export function localSuggestion(form: ConnectForm, local: Probe | null): boolean {
  return form.step === 'server' && !form.preset && !form.url.trim() && local?.kind === 'found';
}
