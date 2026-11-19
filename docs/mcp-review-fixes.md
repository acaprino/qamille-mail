# Correzioni delle review MCP e installazione assistenti

Le dieci anomalie segnalate nelle due review sono state corrette. Sono inclusi i casi collegati trovati durante la verifica: permessi calendario vuoti, installazione incrementale dei componenti ChatGPT, cambio account durante la sincronizzazione e nomi degli allegati importati via MCP.

## Comportamenti corretti

1. **Permessi e file.** I profili Svelte vengono copiati da uno snapshot; un caricamento fallito blocca il salvataggio. `null` indica tutti i calendari, `[]` nessuno. Descrittori e contenuti MCP gestiti non possono essere reimportati per rimuovere il legame con i permessi originali, anche con una cartella di importazione più ampia.
2. **Bozze e invio.** La prima sincronizzazione conserva le modifiche locali e registra il contenuto inviato come riferimento anche se si perde la risposta. I conflitti restano visibili. Una bozza collegata al cloud o in sincronizzazione non può cambiare account; la risposta del provider verifica account e chiave prima di collegare gli identificativi remoti. Un invio dall’esito incerto restituisce un errore esplicito senza ripetere la spedizione. I nuovi snapshot degli allegati conservano il nome originale, usando cartelle distinte per i nomi duplicati.
3. **Calendario e log.** Gli aggiornamenti delle occorrenze successive validano il nuovo evento e le opzioni prima di troncare la serie esistente. Il log registra una sola volta anche gli errori delle operazioni che terminano dopo la risposta asincrona, senza salvare corpi, indirizzi o testo degli errori.
4. **Installazione e impostazioni.** Su Windows gli aggiornamenti dei file usano handle esclusivi e pubblicazione senza sostituire modifiche concorrenti. Il backup verificato può essere recuperato dopo un’interruzione quando manca la destinazione; un file creato esternamente viene conservato. Aggiungere MCP o skill al plugin ChatGPT conserva l’altro componente. I client diretti installati mostrano “Configurato”. I profili invalidi non modificano l’avvio automatico; un errore nel salvataggio ripristina lo stato precedente del launcher.

Il recupero dell’installer è per singolo file. Le prove di avvio automatico verificano gli errori restituiti durante il salvataggio; non simulano un arresto del sistema fra aggiornamenti di file diversi.

## Verifiche

I test usano account fittizi, risposte dei provider simulate e cartelle isolate. Nessuna integrazione viene installata nei profili reali degli assistenti e nessuna email viene inviata.

- Interfaccia: `checks/fix-all-ui-settings.log`, `checks/fix-all-ui-installers.log`, `checks/fix-all-ui-mcp.log`.
- Build web: `checks/fix-all-ui-final-build.log`.
- Test Rust: `checks/fix-all-rust-final.log`.
- Build Windows: `checks/fix-all-desktop-build.log` e `checks/fix-all-server-build.log`.
- Binari consegnati: `checks/fix-all-desktop-smoke.log`, `checks/fix-all-server-smoke.log` e `checks/fix-all-startup-smoke.log`.

Risultato della suite Rust completa: **402 superati, 0 falliti, 6 ignorati**. Nessun test escluso dalla selezione.

La build web termina con **0 errori e 0 avvisi**. I **10 scenari browser** verificano caricamento dei profili, permessi senza ampliamenti, distinzione fra tutti e nessun calendario, errori recuperabili, installazione e funzionalità MCP esistenti.

Entrambi i binari consegnati superano **6 scenari ciascuno**: invio simulato idempotente, calendario locale e divieto di invio del profilo limitato, blocco del reimport gestito con radice ampia, nomi originali negli allegati importati e accodati, revoca a processo attivo e protocollo del processo di estrazione. Passano anche l’estrazione della skill incorporata e l’avvio MCP con un profilo chiamato `skill`.

Gli eseguibili aggiornati e le impronte SHA-256 si trovano in `artifacts/mcp/`.
