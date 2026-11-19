// Storage behaviour for the isolated browser fixture. Never reads native mail files.
import type { AccountStoragePolicy, AccountStorageStatus, Email, Store } from './types';

const policiesKey = 'qamille:preview-account-storage:v1';
const encoder = new TextEncoder();
const encodedSize = (value: unknown) => encoder.encode(JSON.stringify(value)).byteLength;
const legacyPolicy = (): AccountStoragePolicy => ({
  limit: { mode: 'unlimited' },
  keep_all_offline: false,
});

function validatePolicy(value: AccountStoragePolicy): AccountStoragePolicy {
  if (!value || typeof value.keep_all_offline !== 'boolean' || !value.limit)
    throw new Error('Choose valid account storage settings.');
  if (value.limit.mode !== 'limited' && value.limit.mode !== 'unlimited')
    throw new Error('Choose limited or unlimited storage.');
  if (value.limit.mode === 'limited' &&
      (!Number.isSafeInteger(value.limit.max_bytes) || value.limit.max_bytes <= 0))
    throw new Error('Enter a positive storage limit.');
  return structuredClone(value);
}

export function initializePreviewStorage(store: Store) {
  let saved: Record<string, AccountStoragePolicy> = {};
  try { saved = JSON.parse(localStorage.getItem(policiesKey) || '{}'); } catch { /* Fresh preview. */ }
  for (const account of store.accounts) {
    try {
      account.storage_policy = validatePolicy(saved[account.email] ?? account.storage_policy ?? legacyPolicy());
    } catch {
      account.storage_policy = legacyPolicy();
    }
  }
  // Restored limits apply only to replaceable sample payloads; unique data is preserved.
  for (const account of store.accounts) {
    try { applyPreviewStorage(store, account.id, account.storage_policy, true, false); } catch { /* Show over-limit status. */ }
  }
}

function contentBytes(email: Email): number {
  if (email.remote_only) return 0;
  return encoder.encode(email.body).byteLength + email.attachments.reduce((total, attachment) =>
    total + (attachment.path ? attachment.size : encoder.encode(attachment.inline_text ?? '').byteLength), 0);
}
function metadataBytes(email: Email): number {
  return encodedSize({
    ...email,
    body: '',
    attachments: email.attachments.map((attachment) => ({ ...attachment, path: null, inline_text: null })),
  });
}
function removable(store: Store, email: Email, policy: AccountStoragePolicy): boolean {
  return !policy.keep_all_offline && !email.remote_only && !!email.message_id &&
    store.accounts.find((account) => account.id === email.account_id)?.kind === 'Gmail' &&
    !store.drafts.some((draft) => draft.reply_to === email.id) &&
    !store.undo_actions.some((action) => action.operation.kind === 'mail' && action.operation.email_id === email.id);
}
function hasLocalState(email: Email): boolean {
  return !!(email.pinned || email.done || email.muted || email.set_aside || email.priority_thread ||
    email.snoozed_until || email.reminder_at || email.due || email.assigned_to || email.returned_at);
}
export function previewStorageStatus(store: Store, accountId: number): AccountStorageStatus {
  const account = store.accounts.find((account) => account.id === accountId);
  if (!account) throw new Error('Account not found');
  const policy = account.storage_policy;
  const emails = store.emails.filter((email) => email.account_id === accountId);
  const metadata_bytes = emails.reduce((total, email) => total + metadataBytes(email), 0);
  const content_bytes = emails.reduce((total, email) => total + contentBytes(email), 0);
  const draft_bytes = store.drafts.filter((draft) => draft.account_id === accountId)
    .reduce((total, draft) => total + encodedSize(draft), 0);
  const used_bytes = metadata_bytes + content_bytes + draft_bytes;
  const reclaimable_bytes = emails.filter((email) => removable(store, email, policy))
    .reduce((total, email) => total + contentBytes(email) + (hasLocalState(email) ? 0 : metadataBytes(email)), 0);
  const remote_only_messages = emails.filter((email) => email.remote_only).length;
  return {
    account_id: accountId,
    policy: structuredClone(policy),
    used_bytes,
    protected_bytes: used_bytes - reclaimable_bytes,
    reclaimable_bytes,
    metadata_bytes,
    content_bytes,
    draft_bytes,
    remote_only_messages,
    offline_complete: account.kind === 'Sample' && remote_only_messages === 0,
    over_limit: policy.limit.mode === 'limited' && used_bytes > policy.limit.max_bytes,
  };
}

export function applyPreviewStorage(
  store: Store,
  accountId: number,
  requested: AccountStoragePolicy,
  confirmCleanup: boolean,
  persist = true,
): AccountStorageStatus {
  const policy = validatePolicy(requested);
  const working = structuredClone(store);
  const account = working.accounts.find((account) => account.id === accountId);
  if (!account) throw new Error('Account not found');
  account.storage_policy = policy;
  let status = previewStorageStatus(working, accountId);
  const admissionLimit = policy.limit.mode === 'limited'
    ? policy.limit.max_bytes - Math.floor(policy.limit.max_bytes / 10)
    : Infinity;
  if (status.used_bytes > admissionLimit) {
    if (status.protected_bytes > admissionLimit)
      throw new Error('The limit is smaller than protected mail and drafts. Increase the limit or change offline protection.');
    if (!confirmCleanup)
      throw { code: 'cleanup_confirmation_required', message: `This change needs to clear at least ${status.used_bytes - admissionLimit} bytes of local mail from this account, keeping room for safe saves.` };
    const timestamp = (email: Email) => email.date_missing || email.date <= 0 ? Infinity : email.date;
    const candidates = working.emails.filter((email) => email.account_id === accountId && removable(working, email, policy))
      .sort((a, b) => timestamp(a) - timestamp(b) || a.id - b.id);
    for (const email of candidates) {
      if (status.used_bytes <= admissionLimit) break;
      if (hasLocalState(email)) {
        email.body = '';
        email.remote_only = true;
        email.storage_evicted = true;
        email.attachments = email.attachments.map((attachment) => ({ ...attachment, path: null, inline_text: null }));
      } else {
        working.emails = working.emails.filter((candidate) => candidate.id !== email.id);
      }
      status = previewStorageStatus(working, accountId);
    }
    if (status.used_bytes > admissionLimit) throw new Error('Protected mail cannot fit in this limit. Increase the limit.');
  }
  if (persist) {
    const saved = Object.fromEntries(working.accounts.map((account) => [account.email, account.storage_policy]));
    try { localStorage.setItem(policiesKey, JSON.stringify(saved)); }
    catch { throw new Error('The browser could not save these preview settings.'); }
  }
  store.accounts = working.accounts;
  store.emails = working.emails;
  return status;
}
