---
adr_id: ADR-0005
version: 1
title: Un solo controllo di archiviazione per account
status: Accepted
author: Alfio Caprino
scope:
  paths:
    - desktop/src/components/Settings.svelte
    - desktop/src/components/AccountStorage.svelte
    - app/src/domain/storage.rs
    - app/src/domain/mail_cache.rs
    - app/src/domain/model.rs
    - app/src/application/mail_sync.rs
  modules:
  domains:
    - storage
    - offline-mail
    - settings
governs_symbols:
depends_on:
created_from_task: unifica-archivio-offline
proposed_on: 2026-11-04
proposed_at_sha: null
accepted_on: 2026-11-04
accepted_at_sha: null
last_verified_sha: null
retired_on: null
retired_reason: null
supersedes:
superseded_by: null
changes:
---

# ADR-0005: Un solo controllo di archiviazione per account

<!-- adr-header:start -->
Version 1 · Accepted 2026-11-04 before the first commit · proposed 2026-11-04 by Alfio Caprino
<!-- adr-header:end -->

## Context and Problem Statement

Le Impostazioni del desktop hanno due sezioni che regolano la stessa cosa, cioè quanta posta resta sul computer:

- **Archivio locale** (`desktop/src/components/AccountStorage.svelte:167`): per account, limite in GB oppure illimitato, più l'opzione "Tieni tutta la posta offline". È applicato da `storage::enforce_account` (`app/src/domain/storage.rs`).
- **Posta offline** (`desktop/src/components/Settings.svelte`, sezione `Offline mail`): globale per gli account Gmail, limite per età (`offline_days`) e per spazio (`offline_max_mb`).

Nel backend la seconda è già disattivata: `mail_cache::enforce` applica solo il limite per account e il commento dice che le vecchie impostazioni globali sono inerti (`app/src/domain/mail_cache.rs:34-40`). `prune`, che le applicava, viene chiamata solo dai suoi test. Un test di integrazione garantisce che non cancellino nulla (`app/tests/support/storage.rs:130`). Unico effetto rimasto: cambiarle fa partire una sincronizzazione (`app/src/application/mail_sync.rs:142`). L'interfaccia e `desktop/README.md` descrivono ancora un comportamento che non esiste. La specifica `docs/posta-online-specifica.md:175` prevede già il solo modello per account.

Domanda: quale impostazione decide quanta posta resta sul computer?

## Decision Drivers

- Due impostazioni che si influenzano a vicenda confondono l'utente (osservazione dell'utente, 2026-11-04).
- Un controllo che non ha effetto non deve comparire nell'interfaccia.
- I file di configurazione esistenti devono aprirsi senza errori e senza cancellare posta.

## Decision

1. L'unico controllo è **Archivio locale**, per account: limite in GB oppure illimitato, più "Tieni tutta la posta offline".
2. La sezione **Posta offline** viene rimossa dalle Impostazioni, insieme ai testi e alla documentazione che la descrivono.
3. I campi `offline_days` e `offline_max_mb` vengono rimossi dal modello `Settings` (Rust e TypeScript). Un file salvato che li contiene si apre comunque: i campi sconosciuti vengono ignorati e al salvataggio successivo spariscono.
4. Si eliminano il codice morto che li leggeva: `mail_cache::prune` con i suoi test e la risincronizzazione al loro cambio in `mail_sync.rs`.

## Verified Interview Examples

- **EX-001** (nominal case): Input: apro Impostazioni -> Expected: l'elenco delle sezioni contiene "Archivio locale" e non contiene "Posta offline".
- **EX-002** (boundary case): Input: `store.json` esistente con `"offline_days": 30, "offline_max_mb": 1024` -> Expected: l'app si apre, nessun testo di messaggio viene rimosso, il limite dell'account resta quello di Archivio locale.
- **EX-003** (rejection case): Input: salvo le impostazioni generali dopo l'apertura di quel file -> Expected: il file salvato non contiene più `offline_days` né `offline_max_mb`; nessuna pulizia della posta parte per questo salvataggio.

## Rejected Alternatives

- **RA-001**: riattivare "Posta offline" accanto ad Archivio locale; due limiti sulla stessa posta si contraddicono, che è il problema da risolvere.
- **RA-002**: unire le sezioni aggiungendo un limite per età per account; funzione nuova non richiesta ora. Se servirà, sarà una nuova decisione.
- **RA-003**: lasciare i campi nel modello come inerti; codice e test che proteggono una funzione inesistente.

## Enforceable Invariants

- **INV-001**: la quantità di posta tenuta sul computer dipende solo da `Account.storage_policy`.
- **INV-002**: aprire un file di configurazione con i vecchi campi globali non cancella posta locale.
- **INV-003**: le Impostazioni non mostrano controlli che il backend non applica.

## Consequences

- Rimossi: blocco `Offline mail` in `Settings.svelte`, voci di traduzione, campi in `types.ts` e `preview.ts`, test `desktop/tests/offline-cache.mjs` riscritto o eliminato, campi in `google-sign-in.mjs`.
- Rimossi in Rust: campi `Settings.offline_days`/`offline_max_mb`, `mail_cache::prune` e test, controllo `limits` in `mail_sync.rs`.
- `desktop/README.md`: la sezione su Settings → Offline mail viene sostituita dalla descrizione di Archivio locale.
- Prototipo Doop: la sezione Posta offline viene tolta anche lì.

## Confirmation

- Test Rust: caricamento di uno `store.json` con i vecchi campi senza perdita di posta (EX-002, INV-002).
- Test desktop: le sezioni delle Impostazioni non includono "Posta offline" (EX-001).
- `cargo test` e i test desktop esistenti di Archivio locale (`desktop/tests/account-storage.mjs`) passano.
