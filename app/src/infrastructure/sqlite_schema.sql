-- Qamille mail store, schema version 3 (ADR-0007 decision 1).
-- Applied by `SqliteStoreRepository::open` inside one IMMEDIATE transaction; every
-- statement is idempotent so two processes opening the same file cannot collide.
-- Version 2 guarantees `undo_actions.retry_at_ms` / `attempts` (ADR-0007 decision 8):
-- databases recorded as version 1 gain the missing columns through ALTER TABLE in
-- `sqlite_store::migrate`, which keeps every row.
-- Version 3 adds `change_feed` (Task 4.3, cross-process reactivity): the table is
-- created by this idempotent script, so a version 2 database gains it on the next
-- open and records 3; no ALTER is involved.
--
-- Conventions
--   * Each record table has `seq INTEGER PRIMARY KEY` (insertion order, which is the
--     order of the in-memory vector) and a separate `id` that is the record identity.
--     Ids are u64 in the model and are stored as the same 64 bits in a signed INTEGER.
--   * Child rows carry `position` so list order survives the round trip.
--   * Managed file references are stored portable (`workspace:/...`), never absolute.
--   * Nested value objects (index progress, storage policy, undo operation, ...) are
--     JSON text columns; they are not queried, only carried.

CREATE TABLE IF NOT EXISTS meta (
    name  TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- One row: the whole `Settings` document.
CREATE TABLE IF NOT EXISTS settings (
    id   INTEGER PRIMARY KEY CHECK (id = 1),
    json TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS accounts (
    seq                   INTEGER PRIMARY KEY,
    id                    INTEGER NOT NULL UNIQUE,
    provider_id           TEXT NOT NULL,
    search_index          TEXT NOT NULL,
    name                  TEXT NOT NULL,
    email                 TEXT NOT NULL,
    color                 TEXT NOT NULL,
    kind                  TEXT NOT NULL,
    imap_host             TEXT NOT NULL,
    imap_port             INTEGER NOT NULL,
    smtp_host             TEXT NOT NULL,
    smtp_port             INTEGER NOT NULL,
    username              TEXT NOT NULL,
    oauth_client_id       TEXT NOT NULL,
    signature             TEXT NOT NULL,
    last_signature        INTEGER,
    archive_folder        TEXT NOT NULL,
    trash_folder          TEXT NOT NULL,
    last_sync             INTEGER,
    fetch_count           INTEGER NOT NULL,
    storage_policy        TEXT NOT NULL,
    offline_sync_complete INTEGER NOT NULL,
    offline_sync_error    TEXT,
    history               TEXT NOT NULL DEFAULT '{}'
);

CREATE TABLE IF NOT EXISTS emails (
    seq                  INTEGER PRIMARY KEY,
    id                   INTEGER NOT NULL UNIQUE,
    account_id           INTEGER NOT NULL,
    folder               TEXT NOT NULL,
    thread_id            TEXT NOT NULL,
    message_id           TEXT NOT NULL,
    local_sent           INTEGER NOT NULL,
    remote_deleted       INTEGER NOT NULL,
    outgoing_fingerprint TEXT NOT NULL,
    uid                  INTEGER,
    remote_folder        TEXT,
    remote_uid_validity  INTEGER,
    remote_content_hash  TEXT NOT NULL,
    remote_global_id     INTEGER,
    from_name            TEXT NOT NULL,
    from_addr            TEXT NOT NULL,
    recipients_to        TEXT NOT NULL,
    recipients_cc        TEXT NOT NULL,
    subject              TEXT NOT NULL,
    body                 TEXT NOT NULL,
    remote_only          INTEGER NOT NULL,
    storage_evicted      INTEGER NOT NULL,
    date                 INTEGER NOT NULL,
    date_missing         INTEGER NOT NULL,
    read                 INTEGER NOT NULL,
    starred              INTEGER NOT NULL,
    pinned               INTEGER NOT NULL,
    done                 INTEGER NOT NULL,
    muted                INTEGER NOT NULL,
    set_aside            INTEGER NOT NULL,
    replied              INTEGER NOT NULL,
    priority_thread      INTEGER NOT NULL,
    category             TEXT NOT NULL,
    snoozed_until        INTEGER,
    reminder_at          INTEGER,
    due                  INTEGER,
    assigned_to          TEXT,
    classified           INTEGER NOT NULL,
    returned_at          INTEGER,
    bulk                 INTEGER NOT NULL,
    auto_submitted       INTEGER NOT NULL,
    calendar             INTEGER NOT NULL,
    search_index         TEXT NOT NULL,
    in_reply_to          TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS emails_by_folder ON emails (account_id, folder, date);
CREATE INDEX IF NOT EXISTS emails_by_thread ON emails (account_id, thread_id);
CREATE INDEX IF NOT EXISTS emails_by_message_id ON emails (message_id);

CREATE TABLE IF NOT EXISTS email_labels (
    email_id INTEGER NOT NULL REFERENCES emails (id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    label    TEXT NOT NULL,
    PRIMARY KEY (email_id, position)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS email_labels_by_label ON email_labels (label);

CREATE TABLE IF NOT EXISTS remote_locations (
    email_id   INTEGER NOT NULL REFERENCES emails (id) ON DELETE CASCADE,
    position   INTEGER NOT NULL,
    folder     TEXT NOT NULL,
    uid        INTEGER NOT NULL,
    uid_validity INTEGER,
    PRIMARY KEY (email_id, position)
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS attachments (
    email_id    INTEGER NOT NULL REFERENCES emails (id) ON DELETE CASCADE,
    position    INTEGER NOT NULL,
    id          INTEGER NOT NULL,
    name        TEXT NOT NULL,
    size        INTEGER NOT NULL,
    mime        TEXT NOT NULL,
    path        TEXT,
    inline_text TEXT,
    PRIMARY KEY (email_id, position)
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS drafts (
    seq                INTEGER PRIMARY KEY,
    id                 INTEGER NOT NULL UNIQUE,
    account_id         INTEGER NOT NULL,
    to_addrs           TEXT NOT NULL,
    cc_addrs           TEXT NOT NULL,
    bcc_addrs          TEXT NOT NULL,
    request_id         TEXT NOT NULL,
    remote_id          TEXT,
    remote_revision    TEXT,
    cloud_content_hash TEXT NOT NULL,
    cloud_sync_key     TEXT NOT NULL,
    cloud_sync_pending INTEGER NOT NULL,
    subject            TEXT NOT NULL,
    body               TEXT NOT NULL,
    reply_to           INTEGER,
    send_at            INTEGER,
    attachments        TEXT NOT NULL,
    updated            INTEGER NOT NULL,
    format             TEXT NOT NULL,
    signature_id       INTEGER,
    quote              TEXT NOT NULL
);

-- Durable deliveries; the embedded draft is a JSON document (a snapshot, not a live row).
CREATE TABLE IF NOT EXISTS outbox (
    seq               INTEGER PRIMARY KEY,
    id                INTEGER NOT NULL UNIQUE,
    delivery_key      TEXT NOT NULL,
    request_hash      TEXT NOT NULL,
    draft             TEXT NOT NULL,
    plain             TEXT NOT NULL,
    html              TEXT,
    sender_name       TEXT NOT NULL,
    attachment_bytes  INTEGER NOT NULL,
    status            TEXT NOT NULL,
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL,
    send_at           INTEGER NOT NULL,
    requested_send_at INTEGER,
    error             TEXT,
    owner_pid         INTEGER,
    requires_mcp_send INTEGER NOT NULL,
    source_profile    TEXT
);
CREATE INDEX IF NOT EXISTS outbox_by_status ON outbox (status, send_at);

CREATE TABLE IF NOT EXISTS undo_actions (
    seq           INTEGER PRIMARY KEY,
    id            INTEGER NOT NULL UNIQUE,
    operation     TEXT NOT NULL,
    subject       TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    expires_at_ms INTEGER NOT NULL,
    status        TEXT NOT NULL,
    error         TEXT,
    retry_at_ms   INTEGER,
    attempts      INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS events (
    seq          INTEGER PRIMARY KEY,
    id           INTEGER NOT NULL UNIQUE,
    title        TEXT NOT NULL,
    starts_at    INTEGER NOT NULL,
    ends_at      INTEGER NOT NULL,
    location     TEXT NOT NULL,
    attendees    TEXT NOT NULL,
    meeting_link TEXT,
    conference   TEXT NOT NULL,
    rsvp         TEXT NOT NULL,
    notes        TEXT NOT NULL,
    color        TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS meetings (
    seq           INTEGER PRIMARY KEY,
    id            INTEGER NOT NULL UNIQUE,
    title         TEXT NOT NULL,
    date          INTEGER NOT NULL,
    duration_secs INTEGER NOT NULL,
    audio_path    TEXT,
    transcript    TEXT NOT NULL,
    notes         TEXT NOT NULL,
    summary       TEXT NOT NULL,
    key_points    TEXT NOT NULL,
    action_items  TEXT NOT NULL,
    event_id      INTEGER
);

CREATE TABLE IF NOT EXISTS signatures (
    seq      INTEGER PRIMARY KEY,
    id       INTEGER NOT NULL UNIQUE,
    name     TEXT NOT NULL,
    body     TEXT NOT NULL,
    modified INTEGER NOT NULL,
    html     INTEGER NOT NULL
);

-- Files waiting for deletion after a cache change, in the order they were queued.
CREATE TABLE IF NOT EXISTS cache_removals (
    position INTEGER PRIMARY KEY,
    path     TEXT NOT NULL
);

-- Cross-process change feed (Task 4.3, ADR-0007 EX-005 / decision D-crossproc):
-- every write transaction of every process appends the changes it committed; the
-- desktop polls rows newer than its last read (at most every 2 s, a cheap WAL
-- read) and synthesizes the local UI events plus a scheduler wake. `pid` is the
-- writing process, so the poller skips its own process's rows: their writers
-- already announced those changes themselves. Pruned by the maintenance job.
CREATE TABLE IF NOT EXISTS change_feed (
    seq        INTEGER PRIMARY KEY,
    kind       TEXT NOT NULL,
    account_id INTEGER,
    ids        TEXT NOT NULL DEFAULT '[]',
    pid        INTEGER NOT NULL,
    created_ms INTEGER NOT NULL
);
