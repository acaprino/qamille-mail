# Registrazioni: protezione e recupero dopo crash

Aggiornamento del 2 novembre 2026. Priorità richiesta: conservare l'audio già acquisito e renderlo recuperabile anche quando interfaccia, processo o indice della riunione si interrompono.

## Protezioni implementate

| Punto | Comportamento |
| --- | --- |
| Audio a blocchi | Ogni sorgente scrive WAV PCM16 di massimo **1 secondo**. Anche un pacchetto breve viene scaricato dopo circa 900 ms di attesa quando disco e scheduler rispondono. Le code sono limitate; non si conserva l'intero meeting in memoria. |
| Conferma del blocco | Header e campioni sono sincronizzati sul file, seguiti da ricevuta SHA-256 con identità, durata e dimensione. La pubblicazione non può sostituire un file già presente. Su Windows usa `MoveFileExW` con `WRITE_THROUGH` e senza `REPLACE_EXISTING`; non richiede hard link. |
| Indici ricostruibili | `capture.json` e `session.json` hanno copie precedenti. I checkpoint audio restano autonomi; il manifest completo e la proiezione vengono aggiornati ogni circa 10 secondi di progresso e nelle transizioni rilevanti. Questo evita riscritture complete per ciascuno dei migliaia di blocchi di una riunione lunga. |
| Recupero parziale | Un blocco mancante o con hash errato viene separato dai blocchi utilizzabili. Il report di perdita rimane salvato; l'audio sano può essere riprodotto e trascritto, con stato incompleto esplicito anche dopo nuove aperture. |
| Interfaccia e cancellazione | Il banner mostra durata confermata su disco e recupero disponibile anche dopo il riavvio. Avvio, cancellazione, recupero e avvio lavori condividono un controllo di concorrenza; la pulizia verifica inoltre il lock del recorder. |

## Cosa avviene al riavvio

1. Il repository cerca l'indice valido oppure una copia precedente/staged. Un record illeggibile o di formato più recente non nasconde gli altri meeting e non viene riscritto come se fosse compatibile.
2. Il recorder verifica i blocchi e le ricevute indipendenti. Può ricostruire il manifest da file nativi anche quando gli indici principali mancano; gli import già indicizzati possono essere ricostruiti dai metadati nella sessione.
3. Un `.wav.tmp` con header PCM affidabile viene copiato in un nuovo WAV usando soltanto i frame interi presenti. Non vengono aggiunti campioni per riempire il frammento mancante. Originali danneggiati o temporanei restano in quarantena; i metadati danneggiati restano come `session.damaged-<hash>.json`.
4. **Recupera audio** conferma i blocchi utilizzabili e mantiene l'avviso di registrazione incompleta. Se nessun frame è recuperabile, conserva file e report ed evita una falsa conferma di successo.
5. Prima di aggiornare la rappresentazione compatibile, conserva eventuali metadati più recenti in **Copie conservate durante il recupero**. Titolo, appunti, trascrizione e sintesi sono consultabili e copiabili senza sostituire automaticamente i contenuti attuali.

Una ricevuta permette anche di riconoscere un blocco perso che non era ancora entrato nell'ultimo manifest completo. Metadati semanticamente errati, per esempio una frequenza o una posizione sbagliata nell'indice, vengono confrontati con ricevuta e WAV prima di classificare l'audio come danneggiato.

Gli originali in quarantena non vengono cancellati dal recupero. La cancellazione esplicita del meeting comprende anche quei file; un marcatore durevole `.deleted` impedisce che una vecchia copia dell'indice faccia ricomparire dati già eliminati.

Il manifest viene serializzato attraverso un buffer da 64 KiB, quindi scaricato e sincronizzato prima della pubblicazione. Gli aggiornamenti dei livelli audio pubblicano soltanto il nuovo suffisso dell'elenco dei blocchi; i checkpoint già confermati non vengono ricopiati ogni 20 ms. Gli orfani sani recuperati producono un avviso riassuntivo con conteggio; percorsi e tempi restano nell'elenco completo dei blocchi. Anche gli avvisi delle versioni precedenti vengono compattati. Queste modifiche riducono lavoro e testo ripetuto durante sessioni lunghe senza modificare l'ordine delle conferme su disco.

## Errori durante la registrazione

Un errore della proiezione nello store non rende invisibile il recorder: lo stato live e il comando **Termina** restano disponibili, con avviso sull'indice. Un panic del thread audio viene segnalato come interruzione, senza lasciare un falso stato «Registrazione in corso».

Se una sorgente non riesce a completare un blocco, il flush tenta comunque di salvare la coda dell'altra sorgente. Collisioni di nome non sovrascrivono checkpoint esistenti. Spazio insufficiente, errore dispositivo o overflow interrompono la registrazione e mantengono recuperabili i file confermati.

Le discontinuità interne oltre 40 ms per sorgente vengono conservate in `gaps` e segnate come incomplete; l'eventuale silenzio nel player non nasconde il buco. Gli offset iniziali dei dispositivi e la pausa sono distinti dai gap. UI ed export mostrano gli intervalli.

I nuovi recorder persistono `requires_receipts`. Una ricevuta mancante non elimina il WAV: il percorso entra in `unverified_chunks`, con avviso di integrità non verificata durante recupero, ascolto e trascrizione. I formati legacy/importati restano leggibili senza richiedere ricevute mai previste dal loro formato. Lo snapshot terminale del thread audio non sovrascrive una successiva verifica persistita.

## Risultati AI e lavori interrotti

Trascrizione, sintesi e importazione salvano il risultato terminato in `job-results/<hash-id>.json` prima di applicarlo al meeting. Il checkpoint ha schema, identità del lavoro, revisione sorgente e checksum SHA-256; non contiene impostazioni o credenziali del provider. Anche lo staging `.json.writing`, se completo e verificato, è recuperabile.

All'apertura del meeting, nell'elenco e al riavvio, i risultati pronti vengono applicati una sola volta. Annullamenti e modifiche più recenti impediscono di attivare contenuti superati. La rimozione del checkpoint avviene soltanto dopo che indice principale e copia precedente contengono lo stato finale. Un checkpoint danneggiato resta conservato e non blocca gli altri risultati.

Se il commit finale fallisce, il lavoro compare come interrotto con risultato in attesa di salvataggio: ripristinando l'accesso al disco viene confermato senza una nuova richiesta AI. Se anche il checkpoint su disco fallisce, resta una copia in memoria con avviso esplicito di mantenere l'app aperta. Quella copia non può sopravvivere a un ulteriore arresto del processo: l'audio già confermato resta indipendente.

Creazione del thread fallita e panic del processore o dell'installatore modelli producono uno stato ritentabile. I lavori rimasti a metà prima di produrre il risultato vengono segnati interrotti al riavvio; il nuovo tentativo può riutilizzare le finestre STT già in cache. Non viene rilanciata automaticamente una richiesta al provider.

Il claim del lavoro è ora un lock OS condiviso fra desktop e MCP, mantenuto fino all'uscita effettiva del worker. Il file del lock resta fuori dalla cartella rimovibile del meeting; cancellazione e pulizia rispettano lo stesso claim. Un marker per ID job propaga l'annullamento all'altro runtime senza rileggere l'intero meeting. Un crash libera il lock; il polling e l'avvio rilevano i lavori rimasti senza proprietario. L'annullamento non libera il claim in anticipo mentre una richiesta già partita sta ancora terminando.

## Prove eseguite

**18 arresti reali di processi di test**, senza aprire microfono o loopback:

| Prova | Interruzione e risultato verificato |
| --- | --- |
| 12 arresti audio | Abort e kill dopo metà PCM sincronizzato, WAV sincronizzato, prima/dopo pubblicazione, prima del manifest e dopo il suo staging sincronizzato. Numero di frame e valori dei campioni ricostruiti coincidono con i dati effettivamente scritti. |
| 3 arresti metadati | Kill dopo backup, dopo staging sincronizzato e dopo pubblicazione. Alla riapertura si legge la versione precedente o quella confermata, senza JSON parziale pubblicato. |
| 2 arresti risultati AI | Kill dopo staging sincronizzato e dopo pubblicazione. Il risultato pronto viene letto e verificato senza invocare il provider. |
| 1 arresto con claim attivo | Un altro runtime viene escluso mentre il processo detiene il lock; dopo il kill il claim è acquisibile nuovamente. |
| Corruzione/perdita | Manifest mancanti o non validi, metadati incoerenti, blocco centrale alterato, blocco perso oltre l'ultimo manifest, recupero ripetuto, hash e nomi duplicati. |
| Servizi e UI | Registrazione ancora visibile se la proiezione fallisce; cancellazione rifiutata durante preparazione/cattura; copia degli appunti più recenti preservata; avvisi persistenti, elenco funzionante e export marcato incompleto. |

Verifiche delle correzioni nel progetto completo: **96 test Rust passati** (49 unitari audio/elaborazione/persistenza, 28 servizi meeting, 16 archivio, 1 MCP, 2 architettura). Il processo helper dei test metadati è escluso dal lancio ordinario e viene eseguito dai test che lo terminano deliberatamente. La prova storica di due ore sotto non viene sommata al conteggio corrente. Interfaccia: **11 scenari browser passati**, inclusi gli avvisi per gap e integrità, build web con 0 errori/avvisi Svelte. I [tre rilievi della review sono corretti](meetings-code-review.md).

App desktop ricompilata con successo e **4 verifiche native passate**: applicazione una sola volta di un risultato AI pronto in staging prima del crash, senza chiamare il provider; avvio con entrambi gli indici principali danneggiati e recupero senza modificare i byte dell'audio, anche senza ricevuta e con avviso di integrità nella UI; IPC e riproduzione Blob con la CSP di produzione; chiusura della finestra con salvataggio degli appunti meeting e della bozza email. Il recupero mantiene lo stato incompleto anche dopo il salvataggio. La compilazione Rust segnala soltanto il fallback della cache da hard link a copia e l'avviso già presente di compatibilità futura di `imap-proto`.

```powershell
$env:QAMILLE_TEST_ROOT = Join-Path $PWD 'checks/meetings/test-data'
$env:QAMILLE_DATA = Join-Path $PWD 'checks/meetings/workspace-test'
New-Item -ItemType Directory -Force -Path $env:QAMILLE_TEST_ROOT,$env:QAMILLE_DATA | Out-Null
$env:TEMP = $env:QAMILLE_TEST_ROOT
$env:TMP = $env:QAMILLE_TEST_ROOT
cargo test --manifest-path app/Cargo.toml --no-default-features --lib meeting_ -- --test-threads=1
cargo test --manifest-path app/Cargo.toml --no-default-features --test meetings --test meeting_storage --test meeting_mcp --test layers
npm --prefix desktop run test:meetings
npm --prefix desktop run test:meetings:native
```

Il test HTTP incluso usa un server locale e credenziali finte. Gli ambienti che vietano localhost richiedono l'esecuzione autorizzata del solo test; non è necessario accedere a servizi esterni. Il test nativo usa una cartella Sample propria e audio sintetico silenzioso.

### Due ore sintetiche: esito passato

La prova accelerata precedente alla review ha scritto e recuperato **14.400 blocchi su due sorgenti** a 8 e 16 kHz: **57.600.000 e 115.200.000 frame**, per **346.233.600 byte WAV**. Dopo aver danneggiato entrambi gli indici, tutti i blocchi risultavano disponibili, senza intervalli mancanti; contatori, campioni selezionati, continuità per sorgente e seconda riapertura erano corretti. Non è stata rieseguita per la correzione dei tre rilievi: i nuovi casi sono coperti dalle regressioni mirate e dalla suite corrente.

I [risultati grezzi](../checks/meetings/endurance-result.json) riportano 382,76 s di scrittura e 231,05 s di recupero e verifica in build debug nell'ambiente condiviso. Questa misura precede il raggruppamento degli avvisi, introdotto dopo che la prova ha prodotto un avviso per ciascuno dei 14.400 blocchi. Una regressione successiva su **512 blocchi** verifica conteggio completo, compatibilità degli avvisi precedenti, report breve e riapertura. I tempi non sono un benchmark dei dispositivi reali.

Un tentativo precedente si è fermato per esaurimento dello spazio durante compilazioni parallele. Dopo la pulizia delle sole cache generate, i controlli interessati sono passati. Il comando di verifica richiede ora almeno **1 GB libero** prima di avviare la prova lunga; nessuna registrazione reale è stata usata.

Per ripetere la verifica di affidabilità dalla radice del progetto:

```powershell
pwsh -NoProfile -File checks/meetings/verify-durability.ps1
# Prova aggiuntiva esplicita: due ore sintetiche accelerate, circa 350 MB temporanei.
pwsh -NoProfile -File checks/meetings/verify-durability.ps1 -Endurance
# Diagnosi quando altri moduli dell'app sono in modifica:
pwsh -NoProfile -File checks/meetings/verify-durability.ps1 -Isolated
```

L'harness isolato include i sorgenti reali di audio, repository, dominio e servizio meeting; usa un adattatore in memoria di prova per lo store generale. Non sostituisce la compilazione dell'app desktop.

## Limiti dichiarati

**Un blocco da un secondo non equivale alla promessa di perdere al massimo un secondo.** Al crash possono esserci anche campioni nella coda, nella callback o nei buffer del dispositivo. Il tempo di sincronizzazione dipende dal disco e dallo scheduler. La durata confermata su disco indica l'ultimo punto temporale salvato, non garantisce assenza di lacune precedenti; gli intervalli mancanti noti sono segnalati separatamente.

Le prove coprono crash del processo, guasti simulati sui file e due ore sintetiche accelerate del writer/recupero. Non è stato eseguito uno spegnimento fisico durante la scrittura; sincronizzazione e `WRITE_THROUGH` dipendono comunque da filesystem, driver e dispositivo. Le copie sono sullo stesso disco: non proteggono dalla sua perdita fisica. Restano da misurare dispositivi reali, callback, memoria residente, tempi di flush sotto carico e sessione di due ore a tempo reale.

L'AI non è nel percorso di salvataggio audio. Un errore di elaborazione o del suo salvataggio non elimina i blocchi audio. Non viene dichiarata una garanzia di perdita zero.

Codice principale: `meeting_audio.rs` e `meeting_audio/durability.rs`, `meeting_repository.rs`, `application/meetings.rs`. Il [rapporto generale](meetings-implementazione.md) conserva configurazione di trascrizione, sintesi e riferimento upstream.
