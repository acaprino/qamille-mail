// Settings → Compute → Subscription (spec «Vendita Professional», V6). Shown only when the
// server sells subscriptions. Payment and management happen on the processor's pages in the
// browser: this panel opens them and follows the payment by its session id. No card data, no
// price logic here: plans and state come from the server.
import { useEffect, useRef, useState } from 'react';
import { useFocusAfterStep } from '../lib/focus-after-step';
import Button from '../ui/Button';
import { notify } from '../ui/Toaster';
import { call, errorText } from '../lib/api';
import { isItalian, tr } from '../lib/i18n';
import type { AccountInfo, BillingInfo, PlanInfo, Probe, Subscription } from '../lib/server-connect';
import { QUOTA_KEYS, biggerPlan, exhaustedKeys, exhausted, percent, quotaName, quotaPauseText, quotaResource, renewalText, usageText } from '../lib/subscription-usage';

type BillingPage = { id: string; url: string; expires_at: string };
type CheckoutStatus = { id: string; status: 'open' | 'completed' | 'failed' | 'canceled' | 'expired'; plan_id: string | null };

/** How often the panel asks the server about a payment open in the browser. */
const POLL_MS = 2000;

function day(value: string | null): string {
  if (!value) return '';
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleDateString(isItalian() ? 'it-IT' : 'en-GB', { day: 'numeric', month: 'long', year: 'numeric' });
}

export function price(plan: PlanInfo): string {
  const amount = new Intl.NumberFormat(isItalian() ? 'it-IT' : 'en-GB', { style: 'currency', currency: plan.currency }).format(plan.price_cents / 100);
  return plan.interval === 'year' ? tr(`${amount} all'anno`, `${amount} a year`) : tr(`${amount} al mese`, `${amount} a month`);
}

export function subscriptionText(sub: Subscription | null): string {
  const plan = sub?.plan_name ?? tr('Abbonamento', 'Subscription');
  switch (sub?.status ?? 'none') {
    case 'trialing':
      return tr(`Prova gratuita fino al ${day(sub!.current_period_end)}.`, `Free trial until ${day(sub!.current_period_end)}.`);
    case 'active':
      if (sub!.cancel_at_period_end)
        return tr(`${plan}: attivo fino al ${day(sub!.current_period_end)}, non si rinnova.`, `${plan}: active until ${day(sub!.current_period_end)}, does not renew.`);
      return sub!.current_period_end
        ? tr(`${plan}: attivo, si rinnova il ${day(sub!.current_period_end)}.`, `${plan}: active, renews on ${day(sub!.current_period_end)}.`)
        : tr(`${plan}: attivo.`, `${plan}: active.`);
    case 'past_due':
      return tr(
        `${plan}: il rinnovo non è riuscito. Il servizio sul server (calcolo, assistente e sincronizzazione) resta aperto ancora qualche giorno: aggiorna il pagamento.`,
        `${plan}: the renewal failed. The service on the server (compute, assistant and synchronization) stays open for a few more days: update the payment.`,
      );
    case 'canceled':
      return tr('Abbonamento terminato.', 'Subscription ended.');
    case 'expired':
      return sub!.current_period_end
        ? tr(`Abbonamento scaduto il ${day(sub!.current_period_end)}.`, `Subscription expired on ${day(sub!.current_period_end)}.`)
        : tr('Abbonamento scaduto.', 'Subscription expired.');
    default:
      return tr('Nessun abbonamento.', 'No subscription.');
  }
}

export type SubscriptionPanelProps = {
  /** The session's server, asked for its plans. */
  url: string;
  account: AccountInfo;
  computeMode: 'server' | 'off';
  /** The account or the computation mode changed: the settings reload. */
  onChanged: () => Promise<void> | void;
};

export default function SubscriptionPanel({ url, account, computeMode, onChanged }: SubscriptionPanelProps) {
  const [billing, setBilling] = useState<BillingInfo | null>(null);
  const [waiting, setWaiting] = useState<BillingPage | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const section = useRef<HTMLElement>(null);
  useFocusAfterStep(section, waiting?.id ?? null);
  const live = useRef(true);
  useEffect(() => () => void (live.current = false), []);
  useEffect(() => {
    void call<Probe>('server_probe', { url })
      .then((probe) => live.current && probe.kind === 'found' && setBilling(probe.info.billing ?? null))
      .catch(() => {});
  }, [url]);

  // A payment open in the browser: the server learns the outcome from the processor.
  useEffect(() => {
    if (!waiting) return;
    let stopped = false;
    const timer = setInterval(() => {
      void call<CheckoutStatus>('server_checkout_status', { id: waiting.id })
        .then(async (status) => {
          if (stopped || !live.current || status.status === 'open') return;
          stopped = true;
          setWaiting(null);
          if (status.status === 'completed') {
            notify(tr('Abbonamento attivo: il calcolo passa al server.', 'Subscription active: compute moves to the server.'));
            if (computeMode === 'off') await call('compute_set_mode', { mode: 'server' }).catch(() => {});
          } else {
            setError(
              status.status === 'failed'
                ? tr('Il pagamento non è riuscito. Puoi riprovare.', 'The payment did not go through. You can try again.')
                : tr('Pagamento non completato.', 'Payment not completed.'),
            );
          }
          await onChanged();
        })
        .catch(() => {});
    }, POLL_MS);
    return () => {
      stopped = true;
      clearInterval(timer);
    };
  }, [waiting, computeMode, onChanged]);

  async function run(work: () => Promise<void>) {
    setBusy(true);
    setError('');
    try {
      await work();
    } catch (e) {
      if (live.current) setError(errorText(e));
    } finally {
      if (live.current) setBusy(false);
    }
  }

  if (!account.billing_enabled) return null;
  const sub = account.subscription;
  const status = sub?.status ?? 'none';
  const paying = (status === 'active' || status === 'past_due') && sub?.provider !== 'manual';
  const plans = billing?.plans ?? [];

  return (
    <section ref={section} className="compute-subscription" aria-label={tr('Abbonamento', 'Subscription')}>
      <h3>{tr('Abbonamento', 'Subscription')}</h3>
      {billing?.test_mode && (
        <p className="compute-test-mode" role="note">
          {tr('Modalità di prova: pagamenti finti, nessun addebito.', 'Test mode: fake payments, no charge.')}
        </p>
      )}
      <p data-subscription={status}>{subscriptionText(sub)}</p>
      {sub?.usage && (
        <div className="compute-usage">
          <ul aria-label={tr('Consumo del mese', 'Usage this month')}>
            {QUOTA_KEYS.map((key) => {
              const meter = sub.usage![key];
              const name = quotaName(key);
              const text = usageText(key, meter);
              return (
                <li key={key} data-quota={key} data-exhausted={exhausted(meter)}>
                  <span>{tr(name.it, name.en)}</span>
                  <progress max="100" value={percent(meter)} aria-label={tr(`${name.it}: ${text.it}`, `${name.en}: ${text.en}`)} />
                  <span className="muted">{tr(text.it, text.en)}</span>
                </li>
              );
            })}
          </ul>
          {(() => {
            const renewal = renewalText(sub.usage.renews_at);
            return renewal && <p className="field-help">{tr(renewal.it, renewal.en)}</p>;
          })()}
          {exhaustedKeys(sub.usage).map((key) => {
            const pause = quotaPauseText(quotaResource(key), sub.usage!.renews_at);
            return (
              <p key={key} role="status" className="compute-alert" data-quota-paused={key}>
                {tr(pause.it, pause.en)}
              </p>
            );
          })}
          {sub.manageable && exhaustedKeys(sub.usage).some((key) => biggerPlan(plans, key, sub.usage![key].limit)) && (
            <div className="button-row">
              <Button variant="secondary" disabled={busy} onClick={() => void run(async () => void (await call('server_open_billing_portal')))}>
                {tr('Passa a un piano più grande', 'Switch to a bigger plan')}
              </Button>
            </div>
          )}
        </div>
      )}
      {!sub?.entitled && (
        <p className="field-help">
          {tr(
            "Senza abbonamento indicizzazione, ricerca per significato e trascrizione non girano. Restano la posta e la ricerca per parole.",
            'Without a subscription indexing, semantic search and transcription do not run. Mail and the search by words remain.',
          )}
        </p>
      )}
      {!paying && plans.length > 0 && !waiting && (
        <ul className="compute-plans" aria-label={tr('Piani', 'Plans')}>
          {plans.map((plan) => (
            <li key={plan.id}>
              <span>
                <strong>{plan.name}</strong> <span className="muted">{price(plan)}</span>
              </span>
              <Button
                variant={plan === plans[0] ? 'primary' : 'secondary'}
                disabled={busy}
                onClick={() =>
                  void run(async () => {
                    setWaiting(await call<BillingPage>('server_start_checkout', { planId: plan.id }));
                  })
                }
              >
                {tr('Abbonati', 'Subscribe')}
              </Button>
            </li>
          ))}
        </ul>
      )}
      {waiting && (
        <div className="compute-alert" role="status">
          <p>
            {tr(
              'Completa il pagamento nella pagina che si è aperta nel browser: Qamille se ne accorge da solo.',
              'Complete the payment on the page that opened in the browser: Qamille notices by itself.',
            )}
          </p>
          <div className="button-row">
            <Button variant="text" onClick={() => setWaiting(null)}>
              {tr('Smetti di aspettare', 'Stop waiting')}
            </Button>
          </div>
        </div>
      )}
      {sub?.manageable && (
        <div className="button-row">
          <Button variant="secondary" disabled={busy} onClick={() => void run(async () => void (await call('server_open_billing_portal')))}>
            {tr('Gestisci abbonamento', 'Manage subscription')}
          </Button>
          <Button variant="text" disabled={busy} onClick={() => void run(async () => void (await onChanged()))}>
            {tr('Aggiorna', 'Refresh')}
          </Button>
        </div>
      )}
      {error && (
        <p role="alert" className="index-notice">
          {error}
        </p>
      )}
    </section>
  );
}
