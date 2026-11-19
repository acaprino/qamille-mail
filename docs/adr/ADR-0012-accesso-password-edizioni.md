---
adr_id: ADR-0012
version: 1
title: "Accesso al server remoto con username e password; edizione self-hosted ed edizione Professional preimpostata"
status: Proposed
author: Claude Code (su richiesta del proprietario del repository)
scope:
  paths:
    - server/**
    - app/build.rs
    - app/src/edition.rs
    - app/src/domain/server_url.rs
    - app/src/domain/server_account.rs
    - app/src/application/server_account.rs
    - app/src/infrastructure/server_account.rs
    - app/src/infrastructure/secret.rs
    - app/src/domain/model.rs
    - app/src/bin/qamille-mcp.rs
    - desktop/src-tauri/src/main.rs
    - desktop/src-tauri/tauri.conf.json
    - desktop/src/components/ComputeSettings.tsx
    - desktop/src/components/server-connect/**
    - desktop/src/components/Onboarding.tsx
    - Build-Qamille.bat
    - docs/portable.md
  modules:
    - qamille_server
  domains:
    - security
    - auth
    - compute-server
governs_symbols:
depends_on:
  - ADR-0007
  - ADR-0011
created_from_task: accesso-password-edizioni-2026-11-12
proposed_on: 2026-11-12
proposed_at_sha: null
accepted_on: null
accepted_at_sha: null
last_verified_sha: null
retired_on: null
retired_reason: null
supersedes:
superseded_by: null
changes:
---

# ADR-0012: Accesso con username e password, edizione self-hosted ed edizione Professional

<!-- adr-header:start -->
Version 1 · Proposed 2026-11-12 · specifica: `docs/specs/2026-11-12-accesso-password-edizioni-design.md` versione 2 (modifica `docs/specs/2026-11-07-server-calcolo-locale-remoto-design.md`), piani: eseguiti e rimossi il 17 novembre 2026 (storia di git)
<!-- adr-header:end -->

> **17 novembre 2026:** il contenuto resta valido, ma il codice si sposta nell'edizione enterprise secondo [ADR-0014](ADR-0014-core-open-source-ed-enterprise.md).

## Context and Problem Statement

La specifica del server di calcolo prevedeva token personali emessi dall'amministratore e incollati nell'app (D5). Il proprietario vuole vendere Qamille come **Professional**, già collegato al proprio server e con registrazione dall'app, e distribuirlo **open source self-hosted**, con utenze gestite dall'amministratore e indirizzo del server scritto a mano. Un token da conservare non va bene per un cliente, e due modi di entrare sarebbero due logiche. Domanda: con che cosa si entra nel server remoto, e come si distinguono le due edizioni?

## Decision Drivers

- Un solo modo di entrare per tutte le edizioni.
- Nessuna credenziale predefinita e nessun utente nato da solo.
- Chi conosce l'identificativo di qualcuno non deve poterlo chiudere fuori, neanche con richieste in parallelo.
- La modalità «Su questo computer» resta senza account in entrambe le edizioni.
- Il server non conserva contenuti dell'utente.

## Decision

1. **Username (o email) e password** ottengono la **chiave di un dispositivo** (`POST /v1/login`). L'app la tiene nel vault come **sessione** `{url, user_id, username, device_id, device_key}`, un solo valore, e la manda solo a quell'`url`. La password non si salva mai. Il token personale di D5 e l'impostazione `compute_url` spariscono.
2. **Regole dell'accesso:** lunghezza 10–256, argon2id 19 MiB/2/1, provvisoria di 20 caratteri con `403 password_change_required`, stessa risposta `401 invalid_credentials` dopo lo stesso lavoro, rallentamento 250 ms → 4 s con tetto dall'arrivo della richiesta e senza togliere voci ancora nella finestra, mai blocco, massimo di dispositivi in `BEGIN IMMEDIATE`. Il limite per origine si controlla prima dell'attesa e di argon2, dietro un proxy solo con `trusted_proxies`.
3. **Utenze del server:** nascono da `qamille-server user create` sulla macchina, da `/admin` con la chiave amministratore, oppure, solo con `QAMILLE_SIGNUP` e SMTP, dalla registrazione con codice per posta. `/admin` sta su un ascolto separato (`admin_bind`), mai sull'indirizzo pubblico. Utenti, dispositivi e hash in `server.db` (rusqlite).
4. **Due build della stessa app**, distinte solo da `QAMILLE_PRESET_SERVER`, letta dal crate `app/` con `option_env!` e validata da `build.rs` (valore non valido = build fallita):
   - senza: **self-hosted**, primo avvio locale, collegamento da Impostazioni → Calcolo con i passi «Server», «Prima di collegarti», «Accedi»; «Cambia server»; **mai** «Crea account»;
   - con: **Professional**, primo avvio su «Accedi a Qamille» con «Crea account» e «Usa solo su questo computer»; nessun passo «Server», nessun «Cambia server». Installer NSIS per utente, perché i dati stanno accanto all'eseguibile.
5. **Consenso prima delle credenziali:** verso un indirizzo nuovo, prima del consenso parte solo `GET /v1/info`.
6. **Due comandi del server:** `serve` (remoto, account) e `serve --local` (figlio dell'app, solo token per avvio, nessun account).

## Rejected Alternatives

1. **Tenere i token personali accanto alla password:** due modi di entrare, due cose da proteggere.
2. **Indirizzo del Professional in un file accanto all'eseguibile:** lo si cambia a mano e le edizioni diventano uguali.
3. **Indirizzo nelle Impostazioni e chiave nel vault separati:** la chiave può partire verso un indirizzo cambiato.
4. **Professional senza modalità locale:** scartata dal proprietario.
5. **Blocco dell'account dopo N errori:** chiunque conosca l'identificativo chiuderebbe fuori il proprietario.
6. **Registrazione in un passo:** rivela chi ha un account e lascia occupare l'email di altri.
7. **Consenso dopo l'accesso:** credenziali e account nascevano prima del consenso.
8. **Sessioni con cookie:** la webview non parla col server.

## Enforceable Invariants

| ADR | Specifica | Regola |
| --- | --- | --- |
| **INV-001** | INV-12 | Password e codici non si scrivono in chiaro da nessuna parte: database (solo hash), log, errori, risposte, `Debug`, vault, eventi `AppEvent`. |
| **INV-002** | INV-13 | Utente sconosciuto, disattivato, senza password, provvisoria scaduta e password sbagliata sono indistinguibili; registrazione e recupero non rivelano se un'email ha un account. |
| **INV-003** | INV-14 | Nessun blocco per identificativo: la password giusta, da un'origine sotto il suo limite, passa entro 4 s dall'arrivo. |
| **INV-004** | INV-15 | Con la password provvisoria non nasce nessun dispositivo; cambio e registrazione sono una transazione. |
| **INV-005** | A2 | In remoto le rotte di calcolo si aprono solo con la chiave di un dispositivo attivo di un utente attivo; l'installazione dei modelli solo con la chiave amministratore. In locale vale il token per avvio. |
| **INV-006** | INV-16 | Nessun utente e nessuna password predefiniti; la registrazione esiste solo con `QAMILLE_SIGNUP` e SMTP. |
| **INV-007** | INV-17 | Una build con `QAMILLE_PRESET_SERVER` non usa nessun altro server Qamille; una build senza non offre mai la registrazione. Vincolo d'interfaccia, non di sicurezza. |
| **INV-008** | INV-2 | Il server non conserva testo, audio, allegati o vettori dell'utente. |
| **INV-009** | INV-4, INV-10 | La chiave del dispositivo va solo all'`url` della sua sessione; verso un indirizzo nuovo le credenziali partono solo dopo il consenso. |
| **INV-010** | INV-18 | Le rotte `/admin` esistono solo sull'ascolto amministrativo. |

## Consequences

- `qamille-server` ha stato per le utenze (`server.db`) e due comandi di avvio; resta senza contenuti dell'utente.
- D5 ed EX-5 della specifica del server di calcolo sono sostituiti; D1, D7, D8, INV-2, INV-4, INV-10 ed EX-10 sono completati o riformulati. Il passo 4 del server di calcolo e la wave 2 dell'accesso si costruiscono insieme.
- Il Professional richiede SMTP sul server del venditore (registrazione e recupero).
- Il bundle Tauri si attiva solo per NSIS per utente; il portable resta.
- Copiare un `data/` copia l'identità del dispositivo: dichiarato in `docs/portable.md`, da risolvere con la sync.
- Pagamenti, piani e quote commerciali restano un progetto a parte.

## Confirmation

Test del server (`server/tests/`: login, rallentamento con concorrenza e tabella piena, limite per origine e proxy, limite di dispositivi, riga di comando, ascolto amministrativo, registrazione, recupero, segreti nei log), test dell'app su build con e senza `QAMILLE_PRESET_SERVER`, `app/tests/layers.rs` verde, revisione di sicurezza su ogni incremento con codice del server.
