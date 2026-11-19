export type Folder =
  | 'Inbox'
  | 'Tasks'
  | 'Snoozed'
  | 'Drafts'
  | 'Sent'
  | 'Archive'
  | 'Trash'
  | 'Calendar'
  | 'Contacts'
  | 'Meetings'
  | 'Assistant'
  | 'Activity'
  | 'Settings'
  | `label:${string}`;
export interface Account {
  id: number;
  name: string;
  email: string;
  color: number[];
  kind: 'Sample' | 'Gmail';
  provider_id?: string;
  last_sync: number | null;
  signature: unknown;
  storage_policy: AccountStoragePolicy;
}
export type StorageLimit =
  | { mode: 'limited'; max_bytes: number }
  | { mode: 'unlimited' };
export interface AccountStoragePolicy {
  limit: StorageLimit;
  keep_all_offline: boolean;
  /** Gmail's Spam and Trash are listed with the history (W3.1). Absent in old previews. */
  include_spam_trash?: boolean;
}
export interface AccountStorageStatus {
  cleanup_pending?: boolean;
  index_rebuild_required?: boolean;
  account_id: number;
  policy: AccountStoragePolicy;
  used_bytes: number;
  protected_bytes: number;
  reclaimable_bytes: number;
  remote_only_messages: number;
  offline_complete: boolean;
  over_limit: boolean;
  metadata_bytes?: number;
  content_bytes?: number;
  draft_bytes?: number;
}
/** A part inside an attached message; `index` is its position among that message's attachments. */
export interface AttachedPart {
  index: number;
  name: string;
  size: number;
  mime: string;
  message: boolean;
}
/** An attachment that is itself an email (`.eml`, `message/rfc822`), parsed for reading. */
export interface AttachedMessage {
  subject: string;
  from_name: string;
  from_addr: string;
  to: string[];
  cc: string[];
  date: number | null;
  body: string;
  body_html: string;
  attachments: AttachedPart[];
}
export interface Attachment {
  id: number;
  name: string;
  size: number;
  mime: string;
  path: string | null;
  inline_text: string | null;
}
export interface EmailSummary {
  folder?: string;
  date_missing?: boolean;
  match_reason?: string;
  match_terms?: string[];
  id: number;
  message_ids: number[];
  account_id: number;
  from_name: string;
  from_addr: string;
  subject: string;
  snippet: string;
  date: number;
  read: boolean;
  pinned: boolean;
  done: boolean;
  category: string;
  section: string;
  labels: string[];
  attachment_count: number;
  due: number | null;
  /**
   * Root of the conversation, stable when newer mail arrives. Only in the Conversations mode;
   * left out of the data when empty, so a row patched by `read_rows` keeps what the list gave it.
   */
  conversation_id?: number;
  /** The other messages of the conversation in tree order, at most REPLY_CAP; display only. */
  replies?: ConversationReply[];
  /** How many more `read_replies` can load. */
  replies_more?: number;
  /** One of those not yet loaded is unread: a closed conversation must not look quiet. */
  replies_more_unread?: boolean;
}
/**
 * A message shown under a conversation's main row (ADR-0010). It is for reading, not for acting
 * on: the row's `message_ids` stay the messages the open view can act on.
 */
export interface ConversationReply {
  id: number;
  account_id: number;
  /** The message this one answers; null for a root of the tree, also one that answers the main row. */
  parent_id: number | null;
  from_name: string;
  from_addr: string;
  to: string[];
  snippet: string;
  date: number;
  read: boolean;
  folder: string;
}
export interface Email extends Omit<
  EmailSummary,
  | 'snippet'
  | 'section'
  | 'attachment_count'
  | 'message_ids'
  | 'conversation_id'
  | 'replies'
  | 'replies_more'
  | 'replies_more_unread'
> {
  thread_id: string;
  message_id: string;
  /** Message-ID of the direct parent, without the angle brackets; empty or absent when unknown. */
  in_reply_to?: string;
  folder: string;
  body: string;
  /** Sender's HTML alternative, present only while the message is open. */
  body_html?: string;
  to: string[];
  cc: string[];
  attachments: Attachment[];
  snoozed_until: number | null;
  reminder_at: number | null;
  set_aside: boolean;
  returned_at: number | null;
  remote_only?: boolean;
  storage_evicted?: boolean;
  muted?: boolean;
  priority_thread?: boolean;
  assigned_to?: string | null;
}
export interface Draft {
  id: number;
  account_id: number;
  to: string;
  cc: string;
  bcc?: string;
  subject: string;
  body: string;
  reply_to: number | null;
  send_at: number | null;
  attachments: string[];
  updated: number;
  format: 'Plain' | 'Rich' | 'Html';
  signature_id: number | null;
  quote: string;
}
export interface Signature {
  id: number;
  name: string;
  body: string;
  modified: number;
  html: boolean;
}
export interface Settings {
  /** Where the computation runs (access wave 2); absent in old previews. */
  compute_mode?: 'server' | 'off';
  compute_ca_pem?: string | null;
  compute_consented_urls?: string[];
  /** The provider of the server's assistant the person agreed to; written only by the backend. */
  llm_consent?: { url: string; label: string; external: boolean } | null;
  onboarding_done?: boolean;
  time_zone?: string;
  locale?: string;
  undo_seconds: number;
  user_name: string;
  signature: string;
  ai_provider: 'Offline' | 'Anthropic' | 'OpenAiCompatible' | 'Ollama' | 'QamilleServer';
  ai_base_url: string;
  ai_model: string;
  auto_classify: boolean;
  auto_tag: boolean;
  mcp_access: 'ReadOnly' | 'Triage' | 'Send';
  dark: boolean;
  push_labels: boolean;
  google_client_id: string;
  meeting_reminders: boolean;
  reminder_lead_minutes: number;
  /** Spoken language of recordings: a language code such as `it`, or `auto`. */
  meeting_language: string;
  /** `compute`: the compute server of Settings > Compute; `external`: the OpenAI-compatible
   * endpoint of Assistant. Files from before the compute server said `local` and `remote`. */
  meeting_engine: 'compute' | 'external';
  /** Id of the local model: `tiny`, `base` or `small`. */
  meeting_model: string;
  /** Free model name for the remote endpoint; empty uses `whisper-1`. */
  meeting_remote_model: string;
  /** Capture device id; empty follows the system default. */
  meeting_microphone: string;
  /** System audio (loopback) device id; empty follows the system default. */
  meeting_system_audio: string;
}
export interface Workspace {
  regional?: { time_zone: string; locale: string; warning: string | null };
  undo_actions: UndoAction[];
  accounts: Account[];
  counts: Record<string, number>;
  labels: string[];
  settings: Settings;
  has_ai_key: boolean;
  has_google_secret: boolean;
  signatures: Signature[];
  drafts: Draft[];
}
export interface MailPage {
  search?: {
    timezone: string;
    locale: string;
    coverage: string;
    notice: string | null;
    remote_available: boolean;
  } | null;
  items: EmailSummary[];
  total: number;
  next_offset: number | null;
  /** Unread messages of the whole Inbox view, not of this page; null for other folders and searches. */
  unread_total?: number | null;
  /** Rows of the Priority group in the whole Inbox view; null where `unread_total` is. */
  priority_total?: number | null;
}
export interface CalendarEvent {
  id: number;
  title: string;
  start: number;
  end: number;
  location: string;
  attendees: string[];
  meeting_link: string | null;
  conference: string;
  rsvp: 'None' | 'Accepted' | 'Maybe' | 'Declined';
  notes: string;
  color: number[];
}
export interface Meeting {
  id: number;
  title: string;
  date: number;
  duration_secs: number;
  audio_path: string | null;
  transcript: string;
  notes: string;
  summary: string;
  key_points: string[];
  action_items: string[];
  event_id: number | null;
}
export interface SearchHit {
  email_id: number;
  score: number;
  passage: string;
}
export interface Answer {
  text: string;
  sources: SearchHit[];
  meeting_sources?: import('./meetings').MeetingHit[];
}
export interface Store {
  undo_actions: UndoAction[];
  accounts: Account[];
  emails: Email[];
  drafts: Draft[];
  events: CalendarEvent[];
  meetings: Meeting[];
  settings: Settings;
  signatures: Signature[];
  next_id: number;
}

export type UndoOperation =
  | { kind: 'mail'; email_id: number; action: string; value: string | null }
  | { kind: 'send_draft'; draft_id: number }
  | { kind: 'discard_draft'; draft_id: number }
  | { kind: 'schedule_draft'; draft_id: number; send_at: number };
export interface UndoAction {
  id: number;
  operation: UndoOperation;
  subject: string;
  created_at_ms: number;
  expires_at_ms: number;
  status: 'pending' | 'committing' | 'failed';
  error: string | null;
  /** Next attempt of a confirmation that never reached the provider. Optional: the browser preview never retries. */
  retry_at_ms?: number | null;
  /** Retryable failures so far; more than zero makes a closed action a retry row. */
  attempts?: number;
}

/** What the list needs to tell whether a message belongs to a view; the rest of the filter is the query. */
export interface ListFilter {
  folder: Folder;
  account_id: number | null;
  search: string;
  threads: boolean;
}
/**
 * One message as `read_rows` answers it: the summary `list_mail` returns (`date` is the inbox
 * date, the list's sort key) plus three flags that decide whether it belongs to Inbox, Tasks or
 * Snoozed. The flags are always present, `null` / `false` when unset, because the backend
 * serializes an `Option` as `null` and never leaves the key out. The summary alone is not
 * enough: without `snoozed_until` a message that was just snoozed would still match the Inbox.
 * `flushRows` (App.svelte) reloads the list instead of patching it when any row lacks a flag
 * (`carriesFlags`). A message the backend no longer holds is left out of the answer.
 */
export interface MailRow extends EmailSummary {
  snoozed_until: number | null;
  set_aside: boolean;
  reminder_at: number | null;
}

/** Mirror of `SyncPhase` in app/src/application/mail_sync_status.rs (snake_case). */
export type SyncPhase = 'syncing' | 'complete' | 'error';
/**
 * Mirror of `AppEvent` in app/src/application/events.rs, which serializes with
 * `#[serde(tag = "kind", rename_all = "kebab-case")]`: `kind` is the kebab-case variant name and
 * the fields are the variant's fields, flat. Every event names the ids it touched (INV-004).
 */
export type AppEvent =
  | { kind: 'email-changed'; account_id: number; ids: number[] }
  | { kind: 'counts-changed'; account_id: number }
  | { kind: 'undo-changed'; action: UndoAction }
  | { kind: 'undo-removed'; id: number }
  | { kind: 'drafts-changed'; ids: number[] }
  | { kind: 'sync-progress'; account_id: number; phase: SyncPhase; added: number }
  | { kind: 'index-progress'; account_id: number; percent: number }
  | { kind: 'index-waiting'; account_id: number; waiting: boolean }
  /** History backfill (W3.1): messages brought into the list of those in All Mail. */
  | { kind: 'history-progress'; account_id: number; fetched: number; total: number; complete: boolean }
  | { kind: 'history-unread'; account_id: number; count: number }
  /** Search model install (ADR-0007 decision 7): `done` closes success and failure alike. */
  | { kind: 'model-progress'; percent: number | null; done: boolean }
  /** Contacts sync: `null` account is the whole run, an id the book being read. */
  | { kind: 'contacts-sync'; account_id: number | null; running: boolean }
  | { kind: 'notice'; text: string }
  /** The synchronization added, changed or removed mailboxes. */
  | { kind: 'accounts-changed' }
  /** Rows of the activity log were added or corrected; `ids` are their `seq` (centro attività). */
  | { kind: 'activity-changed'; ids: number[] };

/**
 * Mirror of `ActivityEntry` in app/src/domain/activity.rs: one row of the activity log, as the
 * panel and the Activity page receive it. `kind`, `source` and `outcome` serialize lowercase;
 * `code` is a name the UI words (an unknown one is shown generically).
 */
export interface ActivityEntry {
  seq: number;
  at_ms: number;
  kind: 'action' | 'event' | 'error';
  code: string;
  source: 'user' | 'assistant' | 'system';
  account_id: number | null;
  subject: string | null;
  detail: string | null;
  count: number | null;
  ref_kind: string | null;
  ref_id: number | null;
  undo_id: number | null;
  outcome: 'done' | 'undone' | 'failed';
}
/** Which rows the Activity page lists (`ActivityFilter` in the backend). */
export type ActivityFilter = 'all' | 'action' | 'event' | 'error';

// Contacts: shapes of the Rust ContactService replies.
export interface Labeled {
  label: string;
  value: string;
}
export interface ContactFields {
  name: string;
  org: string;
  title: string;
  birthday: string;
  emails: Labeled[];
  phones: Labeled[];
  addresses: Labeled[];
  notes: string;
}
export type ContactSync = 'Synced' | 'Pending' | 'Conflict' | { Error: string };
export interface ContactOrigin {
  key: string;
  book: string;
  label: string;
  writable: boolean;
  sync: ContactSync;
  modified: number;
}
export interface ContactView {
  id: string;
  fields: ContactFields;
  origins: ContactOrigin[];
  hidden: boolean;
}
export interface ContactPage {
  items: ContactView[];
  total: number;
  next_offset: number | null;
}
export type BookKind = 'Local' | 'Google' | 'GoogleOther' | 'Mail';
export interface BookView {
  id: string;
  label: string;
  kind: BookKind;
  writable: boolean;
  sync_enabled: boolean;
  last_sync: number | null;
  write_granted: boolean;
}
export interface DuplicateMember {
  key: string;
  book: string;
  label: string;
  writable: boolean;
  fields: ContactFields;
}
export interface DuplicateView {
  id: string;
  level: 'Certain' | 'Probable' | 'Possible';
  members: DuplicateMember[];
}
export type MergeAction = 'Create' | 'Update' | 'Delete' | { Unavailable: string };
export interface BookAction {
  book: string;
  label: string;
  key: string | null;
  action: MergeAction;
}
export interface MergePreview {
  result: ContactFields;
  actions: BookAction[];
}
export interface BookOutcome {
  book: string;
  ok: boolean;
  error: string | null;
}
export interface MergeOutcome {
  journal_id: number;
  per_book: BookOutcome[];
  recreated: string[];
}
export interface ContactSuggestion {
  name: string;
  address: string;
  books: string[];
}
export interface ContactSettings {
  priority: string[];
  roles: string[];
  last_book: string | null;
  sync_off: string[];
  write_granted: string[];
}
export interface BookSync {
  book: string;
  added: number;
  updated: number;
  removed: number;
  full: boolean;
  skipped: boolean;
  error: string | null;
  consent_required: boolean;
}
export interface ContactSyncReport {
  books: BookSync[];
  writes: BookOutcome[];
}
/** The guide for the server on this PC with Docker (same value as `SERVER_GUIDE_URL` in the app). */
export const SERVER_GUIDE_URL = 'https://github.com/acaprino/qamille-mail/blob/main/docs/server-sul-pc.md';

export const emptyContact = (): ContactFields => ({
  name: '',
  org: '',
  title: '',
  birthday: '',
  emails: [],
  phones: [],
  addresses: [],
  notes: '',
});
