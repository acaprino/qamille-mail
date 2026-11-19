# Code review: recorder e recupero meeting

Data: 2 novembre 2026. Oggetto: hardening audio, repository, lavori AI, comandi desktop e interazione con il runtime MCP. Il primo passaggio è stato una revisione del contenuto disponibile, senza confronto con un commit base. Le sezioni dei rilievi sotto conservano il comportamento osservato prima delle correzioni.

## Stato dopo le correzioni — tre rilievi risolti

| Rilievo | Comportamento corretto | Evidenza |
| --- | --- | --- |
| P1: gap nascosti | Gli intervalli interni oltre 40 ms vengono salvati in `capture.gaps` e rendono l'audio incompleto. Acquisizione, recupero, UI e export mantengono gli intervalli. Scarti iniziali fra dispositivi e pausa non vengono scambiati per gap. | Due test del writer e regressione `recovered_gap_is_reported_as_incomplete`. |
| P2: checksum assente | I nuovi recorder dichiarano `requires_receipts`; i blocchi senza ricevuta rimangono disponibili e vengono elencati in `unverified_chunks`, con avviso persistente. Il controllo avviene anche prima di ascolto e trascrizione. Il polling del recorder fermo non cancella il risultato della verifica. I file legacy/importati rimangono compatibili. | `missing_receipt_keeps_audio_but_marks_integrity_unverified`, `player_detects_a_receipt_lost_after_startup_and_live_polling_keeps_the_warning`, compatibilità legacy e verifica UI. |
| P2: lavori duplicati | Il worker mantiene un lock OS per meeting fino alla sua uscita effettiva. Un altro runtime non avvia elaborazioni o pulizie concorrenti. Un marker di annullamento specifico per job raggiunge l'altro runtime; il lock viene liberato dal sistema dopo un crash. Le letture riconoscono lavori senza più un proprietario e li rendono ritentabili. | Regresso su due servizi, annullamento dall'altro runtime, stato di worker morto e processo di prova terminato mentre detiene il lock. |

Le sette regressioni derivate dalla review sono ora in `app/tests/meetings.rs`, quindi rientrano nella suite principale. L'harness `checks/meetings/review/probes.rs` include gli stessi test canonici: non conserva più aspettative che richiedono il comportamento difettoso.

Verifica della correzione: **96 test Rust passati** (49 unitari, 28 servizi meeting, 16 archivio, 1 MCP, 2 architettura), **18 scenari reali di arresto** coperti dai test, **11 scenari browser**, build web senza errori/avvisi Svelte e **4 verifiche nell'app desktop ricompilata**. La verifica nativa include il WAV privo di ricevuta, mantenuto riproducibile e segnalato nell'interfaccia. Il rapporto di [recupero](meetings-recupero-crash.md) conserva i limiti hardware.

## P1 — Intervalli audio mancanti dichiarati completi

**Posizione:** `app/src/infrastructure/meeting_audio.rs:547–553`; il recupero in `meeting_audio/durability.rs::inspect_capture` non rileva i gap interni per sorgente.

Quando il timestamp di un pacchetto salta in avanti, `drain` scarica il frammento precedente e sposta l'inizio della traccia, senza registrare un intervallo mancante o impostare `incomplete`. Se il buffer era già vuoto, cambia soltanto `start_ms`. Pausa/ripresa sottraggono il tempo di pausa dal clock: un buco interno di un secondo non è una pausa intenzionale da rappresentare come silenzio.

**Riproduzione:** due WAV nativi validi con ricevute corrette coprono 0–1000 e 2000–3000 ms. `load_capture` restituisce `Saved`, `incomplete=false`, nessun avviso e nessun blocco mancante; il mixer produce silenzio per 1000–2000 ms. L'assenza di file corrotti non dimostra la continuità della registrazione.

**Effetto:** perdita di contenuto nascosta dietro una registrazione dichiarata completa. Questo contraddice AUD-04, che richiede di marcare i campioni mancanti senza falsa conferma di completezza.

**Correzione:** rilevare le discontinuità per sorgente oltre la tolleranza, conservarne gli intervalli e segnare il risultato incompleto sia durante acquisizione sia nel recupero. Non confondere i normali scarti fra sorgenti o l'avvio del dispositivo con gap interni della stessa traccia.

## P2 — Ricevuta assente: integrità non verificata senza avviso

**Posizione:** `app/src/infrastructure/meeting_audio/durability.rs:177–181`.

`check_receipt` ritorna successo per qualsiasi WAV se manca il sidecar SHA-256. La compatibilità con registrazioni precedenti e importate è necessaria, ma questa eccezione comprende anche i nuovi blocchi nativi che devono avere una ricevuta sincronizzata prima della pubblicazione.

**Riproduzione:** creare un blocco nativo e la relativa ricevuta, verificarlo, rimuovere il sidecar e alterare un campione PCM mantenendo dimensione e header validi. Il lettore accetta il WAV modificato come `Saved`, `incomplete=false`, senza avvisi; anche il mixer lo usa.

**Effetto:** la perdita del file di integrità disabilita silenziosamente il controllo del contenuto. Il caso riprodotto richiede perdita della ricevuta e modifica del WAV; la sola assenza del checksum deve comunque risultare come integrità non verificata.

**Correzione:** distinguere esplicitamente i formati che richiedono ricevute dai file legacy/importati. Conservare e rendere recuperabile l'audio senza ricevuta, mostrando però l'impossibilità di verificarne l'integrità.

## P2 — Claim dei lavori limitato a una singola istanza del servizio

**Posizione:** `app/src/application/meetings.rs:976–1004`.

L'esclusione usa soltanto `self.controls`, una mappa in memoria creata da ogni `MeetingService::new`. Il controllo e l'aggiunta del job persistito non verificano un claim condiviso. Desktop e MCP costruiscono servizi distinti sullo stesso archivio (`desktop/src-tauri/src/runtime.rs:41` e `app/src/adapters/mcp/tools.rs:1664`). I lock dei file serializzano gli aggiornamenti, ma non coprono la durata dell'elaborazione.

**Riproduzione:** avviare una trascrizione bloccata in un primo servizio, creare un secondo servizio sullo stesso repository e avviare un'altra trascrizione della stessa revisione. Entrambe vengono accettate e il processore viene chiamato due volte. Alla fine un risultato può diventare obsoleto e venire scartato, ma entrambe le elaborazioni sono già state eseguite.

**Effetto:** lavoro e, con provider remoto, richieste/costi duplicati. Il controllo delle revisioni protegge il risultato attivo, non impedisce le due esecuzioni. Anche cancellazione e rimozione devono riconoscere lavori appartenenti all'altro runtime.

**Correzione:** introdurre un claim esclusivo per meeting condiviso fra processi, mantenuto fino alla terminazione del worker e recuperabile dopo crash; integrare lo stesso controllo con cancellazione e pulizia.

## Prove e limiti

Il passaggio iniziale ha riprodotto i tre difetti con fixture sintetiche, senza microfono o servizi esterni. Dopo le correzioni le aspettative sono state invertite e ampliate: il comando seguente esegue i 28 test canonici dei servizi, comprese le sette regressioni relative alla review.

```powershell
$env:QAMILLE_TEST_ROOT = Join-Path $PWD 'checks/meetings/test-data'
$env:TEMP = $env:QAMILLE_TEST_ROOT
$env:TMP = $env:QAMILLE_TEST_ROOT
cargo test --offline --manifest-path checks/meetings/review/Cargo.toml --target-dir checks/meetings/audio-unit/target -- --nocapture --test-threads=1
```

Esito iniziale: tre riproduzioni confermate. Esito dopo la correzione: tutte e sette le regressioni specifiche passano nella suite principale. L'asserzione dell'export usa il formato effettivo `HH:MM:SS`.

Questa verifica non certifica il comportamento dei dispositivi reali o lo spegnimento fisico. La prova sintetica di due ore precedente resta evidenza del writer e del recupero a quella revisione; non è stata rieseguita per queste correzioni, verificate con le regressioni mirate e la suite principale.
