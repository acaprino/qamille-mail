# Spazio per account — stato di implementazione

**Data:** 2 novembre 2026. Riferimento: [specifica completa](posta-online-specifica.md).

## Comportamento disponibile

| Area | Implementazione |
| --- | --- |
| Impostazioni | Ogni account ha «Limitato» con GB modificabili oppure «Illimitato». Default per nuovi account: 2.000.000.000 byte. Gli account legacy senza politica mantengono Illimitato fino alla scelta esplicita dell’utente. |
| Pulizia | Rimuove la mail locale ricostruibile più vecchia dello stesso account; a pari data usa l’ID stabile. Preserva task, bozze, operazioni pendenti e dati senza un riferimento remoto verificato. La riduzione del limite richiede conferma se comporta pulizia. |
| Provider e offline | Sync ordinaria di intestazioni e descrizioni degli allegati; lettura online temporanea. «Conserva tutto offline» attiva passaggi riprendibili, con verifica delle cartelle disponibili e stato incompleto se quota, contenuti o cartelle impediscono di terminare. |
| Ricerca semantica | Nuovo indice senza testo persistito: vettori, riferimenti e hash. Conteggio dello spazio per messaggio/account, verifica della fonte per gli estratti e recupero temporaneo dei corpi. Il modello è un componente comune separato. |
| Integrità | Transazioni con controllo della quota, conferma prima della pulizia richiesta dalle impostazioni, manutenzione ripetibile dopo il commit e protezione delle scritture dallo spazio fisico insufficiente su Windows. |

## Limiti espliciti di questa consegna

Il repository resta quello JSON di compatibilità. La quota contabilizza dati serializzati dell’account, corpi, allegati e record dell’indice; non costituisce ancora un tetto fisico esatto su tutte le copie temporanee e sull’indentazione dell’aggregato condiviso. La separazione dei database per account e la migrazione SQLite della specifica rimangono lavoro distinto.

Le bozze restano gestite dal flusso locale esistente. Questa consegna non introduce bozze cloud, registro cifrato delle revisioni o migrazione automatica degli archivi già presenti. Nessun account reale viene collegato o modificato dalla verifica automatica.

Il download offline avanza fino a 64 MiB di nuovo MIME per passaggio. Messaggi superiori a questa soglia o cartelle non esposte via IMAP impediscono lo stato «completo» e producono un avviso. Gli account protetti non vengono svuotati per rientrare nel limite.

Il nuovo indice senza testo viene scritto alla ricostruzione esplicita o a una pulizia necessaria delle sue voci. La presenza di una vecchia versione non viene nascosta come se il testo fosse già scomparso dal disco.

L'indice resta un file condiviso: la sostituzione atomica richiede spazio temporaneo anche per le voci degli altri account. Se manca il margine, l'indicizzazione si ferma preservando l'indice precedente. Un indice danneggiato viene conservato separatamente e segnalato da ricostruire; queste copie di recupero restano sul disco e non sono incluse nel consumo dell'indice attivo.

La sincronizzazione ordinaria resta limitata alla finestra Inbox configurata. La lettura online usa ancora il recupero dell'originale MIME completo in memoria: il download selettivo delle singole parti e la paginazione di tutto lo storico sono attività successive. Non vengono salvate nuove copie permanenti dei corpi durante questa lettura.

## Verifica

Le prove usano repository temporanei, server IMAP simulati e la modalità di anteprima. Coprono quota limitata/illimitata, isolamento, parità e date mancanti, conferma/annullamento, protezione dei contenuti, collisioni di identità, formato dell’indice e risposte obsolete nella UI. Il collaudo con un account Gmail reale resta una verifica separata dopo il login dell’utente.

Esito della verifica del 2 novembre 2026:

| Verifica | Esito |
| --- | --- |
| Suite Rust senza UI nativa, con dati isolati | 237 test passati, 4 ignorati, 0 falliti. |
| Build Svelte/Vite | Completata; 0 errori e 0 avvisi Svelte. |
| `desktop/tests/account-storage.mjs` | Passato, inclusa risposta di aggiornamento ritardata dopo il salvataggio. |
| Build Tauri con `custom-protocol` | Completata; eseguibile in `app/target/debug/qamille-desktop.exe`. |

Per provarlo, avviare `Start-Qamille.cmd` e aprire **Settings → Local storage**. I log delle verifiche sono in `checks/storage-cargo-tests.log` e `checks/storage-tauri-build.log`.
