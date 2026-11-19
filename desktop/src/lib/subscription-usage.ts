// The consumption of the month, the pauses of a used-up quota, the warning before deleting an
// account that pays, and the Terms of a new account (wave 9). Pure functions without the interface
// language so they can be tested alone: texts are `{ it, en }` pairs the components pass to `tr`.
// The app never changes a subscription (INV-22): it shows what the server says and opens the
// processor's own pages.
import type { LegalInfo, Meter, PlanInfo, Subscription, UsageInfo } from './server-connect';

export type Text = { it: string; en: string };

/** The three meters of `subscription.usage`, in the order the panel shows them. */
export type QuotaKey = 'audio_minutes' | 'embedded_tokens' | 'llm_tokens';
export const QUOTA_KEYS: QuotaKey[] = ['embedded_tokens', 'audio_minutes', 'llm_tokens'];

/** What `error.resource` of a `quota_exceeded` says. */
export type QuotaResource = 'compute' | 'transcription' | 'llm';
const RESOURCE_OF: Record<QuotaKey, QuotaResource> = { embedded_tokens: 'compute', audio_minutes: 'transcription', llm_tokens: 'llm' };

export function quotaResource(key: QuotaKey): QuotaResource {
  return RESOURCE_OF[key];
}

/** The name of a meter. */
export function quotaName(key: QuotaKey): Text {
  switch (key) {
    case 'audio_minutes':
      return { it: 'Trascrizione', en: 'Transcription' };
    case 'embedded_tokens':
      return { it: 'Ricerca per significato', en: 'Semantic search' };
    case 'llm_tokens':
      return { it: 'Assistente', en: 'Assistant' };
  }
}

type Language = 'it' | 'en';
const LOCALE = { it: 'it-IT', en: 'en-GB' } as const;
const amount = (value: number, language: Language) => new Intl.NumberFormat(LOCALE[language]).format(Math.max(0, Math.round(value)));

/** «12 di 600 minuti»: what the account used and what the plan gives. */
export function usageText(key: QuotaKey, meter: Meter): Text {
  const unit = (language: Language) => (key === 'audio_minutes' ? (language === 'it' ? 'minuti' : 'minutes') : language === 'it' ? 'token' : 'tokens');
  return {
    it: `${amount(meter.used, 'it')} di ${amount(meter.limit, 'it')} ${unit('it')}`,
    en: `${amount(meter.used, 'en')} of ${amount(meter.limit, 'en')} ${unit('en')}`,
  };
}

/** The part of the bar to fill, 0 to 100. A limit of zero reads as full: nothing is included. */
export function percent(meter: Meter): number {
  if (meter.limit <= 0) return 100;
  return Math.min(100, Math.max(0, Math.floor((meter.used / meter.limit) * 100)));
}

/** The plan's quota is used up: the server answers `429 quota_exceeded` for it. */
export function exhausted(meter: Meter): boolean {
  return meter.used >= meter.limit;
}

/** The meters at their limit, in the panel's order. */
export function exhaustedKeys(usage: UsageInfo | null | undefined): QuotaKey[] {
  return usage ? QUOTA_KEYS.filter((key) => exhausted(usage[key])) : [];
}

/** «1° dicembre»: the day the month starts again, in UTC as the server counts it. */
function renewalDay(renewsAt: string, language: Language): string | null {
  const date = new Date(renewsAt);
  if (Number.isNaN(date.getTime())) return null;
  const month = new Intl.DateTimeFormat(LOCALE[language], { month: 'long', timeZone: 'UTC' }).format(date);
  const day = date.getUTCDate();
  return language === 'it' ? `${day === 1 ? '1°' : day} ${month}` : `${day} ${month}`;
}

/** «Si azzera il 1° dicembre.» Null when the server gave no date the app can read. */
export function renewalText(renewsAt: string | null | undefined): Text | null {
  const it = renewsAt ? renewalDay(renewsAt, 'it') : null;
  const en = renewsAt ? renewalDay(renewsAt, 'en') : null;
  return it && en ? { it: `Si azzera il ${it}.`, en: `Resets on ${en}.` } : null;
}

/** What a used-up quota stops, and when it comes back. */
export function quotaPauseText(resource: QuotaResource, renewsAt?: string | null): Text {
  const back = (language: Language) => {
    const day = renewsAt ? renewalDay(renewsAt, language) : null;
    if (language === 'it') return day ? `Tornano il ${day}.` : "Tornano all'inizio del mese prossimo.";
    return day ? `They come back on ${day}.` : 'They come back at the start of next month.';
  };
  switch (resource) {
    case 'compute':
      return {
        it: `Indicizzazione e ricerca per significato sono ferme: hai finito i token di ricerca di questo mese. ${back('it')}`,
        en: `Indexing and semantic search are paused: you used all the search tokens of this month. ${back('en')}`,
      };
    case 'transcription':
      return {
        it: `La trascrizione è ferma: hai finito i minuti di audio di questo mese. ${back('it')}`,
        en: `Transcription is paused: you used all the audio minutes of this month. ${back('en')}`,
      };
    case 'llm':
      return {
        it: `L'assistente è fermo: hai finito i token di questo mese. ${back('it')}`,
        en: `The assistant is paused: you used all the tokens of this month. ${back('en')}`,
      };
  }
}

/**
 * The plan to offer when a meter is full: the cheapest one that gives more of it than the plan in
 * use. Null when there is none, or when the server declares no quotas for its plans.
 */
export function biggerPlan(plans: PlanInfo[], key: QuotaKey, currentLimit: number): PlanInfo | null {
  const bigger = plans.filter((plan) => (plan.quotas?.[key] ?? 0) > currentLimit);
  bigger.sort((a, b) => a.price_cents - b.price_cents);
  return bigger[0] ?? null;
}

/**
 * What the dialog «Delete the account» says before its button. A paid subscription (`active` or
 * `past_due`, not granted by hand) is ended by the server at the processor, at once and without
 * an automatic refund: the person is told, and can look at it first. A trial ends with the
 * account. Anything else: nothing to say.
 */
export function deleteWarning(sub: Subscription | null | undefined): Text | null {
  if (!sub) return null;
  if (sub.status === 'trialing' || sub.provider === 'trial') {
    return { it: "La prova finisce con l'account.", en: 'The trial ends with the account.' };
  }
  if ((sub.status === 'active' || sub.status === 'past_due') && sub.provider !== 'manual') {
    return {
      it: "Hai un abbonamento attivo: eliminando l'account il server lo disdice subito presso il processore dei pagamenti. Il periodo già pagato non viene rimborsato in automatico.",
      en: 'You have an active subscription: deleting the account makes the server cancel it at once with the payment processor. The period already paid is not refunded automatically.',
    };
  }
  return null;
}

/** An address of the server's documents that may be opened: `https://` only (INV-26), no credentials. */
export function legalAddress(url: string | null | undefined): string | null {
  // eslint-disable-next-line no-control-regex
  if (!url || /[\s\u0000-\u001f]/.test(url)) return null;
  try {
    const parsed = new URL(url);
    return parsed.protocol === 'https:' && !parsed.username && !parsed.password && parsed.hostname ? parsed.href : null;
  } catch {
    return null;
  }
}

/** The box «I have read the Terms and the privacy notice» appears only on a server that declares them. */
export function needsTermsBox(legal: LegalInfo | null | undefined): boolean {
  return !!legal && (!!legalAddress(legal.terms_url) || !!legalAddress(legal.privacy_url) || !!legal.terms_version?.trim());
}

/**
 * The version sent with a sign-up: the one `GET /v1/info` declared, and only with the box ticked.
 * Never a text of the interface.
 */
export function acceptedTerms(legal: LegalInfo | null | undefined, ticked: boolean): string | null {
  const version = legal?.terms_version?.trim();
  return ticked && version ? version : null;
}

/** «Create account» (and «Continue with Google») wait for the box when the server asks for it. */
export function termsBlock(legal: LegalInfo | null | undefined, ticked: boolean): boolean {
  return needsTermsBox(legal) && !ticked;
}
