# Meeting: implementazione e verifica

Data: 2 novembre 2026. Questo rapporto descrive la prima implementazione desktop e i limiti verificabili; la [specifica](meetings-specifica.md) resta il riferimento di accettazione.

**Aggiornamento affidabilità:** implementato e verificato il [salvataggio a blocchi da un secondo e recupero dopo crash](meetings-recupero-crash.md), comprese copie dei metadati, checksum, recupero parziale e prove con processi terminati forzatamente. Il rapporto dedicato contiene gli esiti più recenti.

## Uso nell'app

Aggiornato al 9 novembre 2026: la pagina **Meeting** è stata resa semplice (specifica: [riunioni semplici](superpowers/specs/2026-11-08-riunioni-semplici-design.md)). Registrare, trascrivere e riassumere non richiedono scelte.

1. **Elenco a sinistra**, in quattro gruppi (un gruppo vuoto non si disegna): **In corso** (la registrazione attiva), **In elaborazione** (trascrizione o sintesi in coda o in esecuzione, con la barra di avanzamento), **Prossime** (eventi del calendario da adesso a sette giorni, con «Partecipa e registra» per quelli che hanno un link; «Iniziata da N min» per quelli già cominciati, e l'elenco segue l'orologio da solo) e **Passate** (registrazioni, più gli eventi delle ultime due settimane che nessuno ha registrato, in grigio come «Non registrata»). Una registrazione collegata a un evento prende il posto dell'evento. Le righe dicono cosa manca: «Da recuperare», «Da trascrivere» (se il motore non può partire, la riga ne dà il motivo e porta a Impostazioni), «Trascrizione non riuscita» con **Riprova**. La ricerca sostituisce l'elenco con i risultati: un clic su una riga della trascrizione apre quel passaggio.
2. **Registra ora** (titolo facoltativo) avvia la registrazione con microfono e audio del computer predefiniti o scelti in Impostazioni. **Pausa/Riprendi** e **Termina** stanno nel dettaglio; l'indicatore resta visibile nelle altre pagine.
3. **Catena automatica.** Con **Termina** (o dopo **Recupera audio** di una registrazione interrotta) parte da sola la trascrizione con le impostazioni salvate; quando il testo è pronto parte la sintesi se un assistente è configurato. Se il motore non è installato o il provider non è configurato non parte nulla: il dettaglio mostra il motivo, il link a Impostazioni e il pulsante **Trascrivi** (le riunioni vecchie non partono da sole all'installazione del modello). **Riprova** dopo una trascrizione fallita o annullata ne avvia una nuova con il motore di adesso, non rilancia il lavoro fallito con le vecchie opzioni; quello di una sintesi fallita rilancia la sintesi.
4. **Dettaglio a destra**: intestazione (titolo, data, durata, partecipanti; menu per rinominare ed eliminare) e due schede, **Sintesi** (punti chiave, decisioni, azioni da spuntare con le fonti, «Copia») e **Trascrizione** (una riga per passaggio con l'orario, in sola lettura, «Copia»). Non ci sono più player, appunti, versioni, correzione dei passaggi o esportazione dalla pagina; i dati restano nell'archivio.
5. **Regola dell'audio.** Quando la trascrizione è completa e ha testo, e la registrazione è pulita (nessun blocco perso o non verificato), l'audio viene eliminato da solo; nel dettaglio si legge «L'audio è stato eliminato dopo la trascrizione». Se la trascrizione è vuota o la registrazione ha interruzioni l'audio resta e il dettaglio dice perché, con **Elimina audio** per chi ha controllato. Un audio non si cancella mai se la trascrizione è fallita, annullata o incompleta. In Impostazioni › Riunioni l'azione una tantum «Elimina l'audio…» (con conferma e totale in MB) pulisce le riunioni già trascritte con una versione di trascrizione completa e testo.
6. **Impostazioni › Riunioni**: lingua della trascrizione, motore (locale, con installazione e annullamento del download del modello, oppure remoto), dispositivi di registrazione, promemoria delle riunioni e pulizia dell'audio. Sono le impostazioni da cui legge il lavoro automatico.

### Endpoint compatibile OpenAI

Aprire **Settings → Assistant**, scegliere **OpenAI-compatible** e salvare endpoint e API key. L'endpoint base deve comprendere l'eventuale prefisso `/v1`; il client aggiunge `/audio/transcriptions`. In Meeting scegliere **Remoto**, lingua e modello STT; il valore iniziale è `whisper-1`. Il campo **Model** nelle impostazioni AI resta il modello testuale per sintesi e assistente.

Un endpoint che espone soltanto chat/completamenti non è sufficiente. Modello STT e formati di risposta devono essere supportati dal server. La stessa configurazione endpoint/credenziale viene condivisa con il provider AI; endpoint e credenziale STT indipendenti non sono ancora un'impostazione dedicata. La chiave resta nel vault cifrato portable, fuori da sessioni, cache e file esportati.

I modelli Whisper compatibili richiedono `verbose_json`; i modelli GPT di trascrizione richiedono `json`. Se il server restituisce soltanto testo, il riferimento temporale copre l'intero blocco inviato, senza inventare tempi per singole parole. Ogni richiesta contiene un WAV mono 16 kHz di massimo 15 secondi. Nessun passaggio automatico dal locale al remoto.

### Modalità locale

Il pulsante d'installazione scarica, su richiesta, un modello multilingue Tiny/Base/Small e il runtime ufficiale CPU **whisper.cpp v1.9.2 Windows x64**. File, revisioni, dimensioni e SHA-256 sono fissati nel catalogo; l'integrità è ricontrollata prima dell'uso. Download annullati non attivano modelli parziali.

Il runtime usa un processo separato e non introduce whisper-rs, CMake o libclang nella build dell'app. La scelta consente di conservare le dipendenze CPAL/ONNX esistenti; non dimostra superiorità rispetto a Parakeet, che non è stato confrontato. Il motore è CPU, senza accelerazione GPU in questa implementazione.

## Componenti implementati

| Area | Codice | Comportamento |
| --- | --- | --- |
| Contratti e servizi | `app/src/domain/meeting.rs`, `application/meeting_ports.rs`, `application/meetings.rs` | Revisioni, idempotenza start, lavori annullabili, protezione da risultati obsoleti, versioni e ricerca. |
| Audio | `app/src/infrastructure/meeting_audio.rs` | Controller dedicato CPAL, microfono e WASAPI loopback, code limitate, livelli, clock comune, pausa, WAV per sorgente e checkpoint. |
| Archivio | `app/src/infrastructure/meeting_repository.rs` | Lock per meeting/processo UI, sostituzione atomica, backup legacy, recupero, tombstone e pulizia dei soli file posseduti. |
| Elaborazione | `app/src/infrastructure/meeting_processing/` | Import progressivo, snapshot originale, STT locale/remoto, cache finestre, sintesi a blocchi con fonti validate ed export. |
| Interfaccia e integrazioni | `desktop/src/components/Meetings.svelte`, `components/meetings/`, `lib/meetings.ts`, comandi Tauri, `application/assistant.rs`, `adapters/mcp.rs` | Pagina semplice: elenco a quattro gruppi con eventi del calendario, dettaglio a due schede, catena automatica trascrizione/sintesi, ricerca, fonti dell'assistente e passaggi MCP. Il comportamento è in `lib/meetings-view.ts` (logica pura) e `lib/meetings-follow.ts` (seguire i lavori in corso). |

### Persistenza e recupero

Ogni meeting V2 usa `<cartella dati>/meetings/<id>/session.json`, `capture.json` e blocchi audio separati. Il recorder conferma blocchi di massimo 1 secondo mediante flush/sync e ricevute SHA-256 indipendenti; i manifest completi vengono aggiornati periodicamente e alle transizioni. Gli import continuano a essere decodificati in blocchi di 5 secondi, mantenendo la copia originale. Le tracce conservano la frequenza nativa; il mix per player/STT viene ricampionato su richiesta. Non viene creato un buffer contenente tutta la registrazione.

Un riavvio identifica lavori interrotti, verifica WAV completi e recupera anche blocchi orfani riconoscibili. **Recupera** persiste lo stato recuperato, che resta tale alle riaperture successive. L'ultimo frammento non confermato può andare perso; l'interfaccia conserva un avviso di incompletezza. Nessun riavvio apre automaticamente i dispositivi o ripete richieste remote.

I risultati AI già terminati vengono salvati in checkpoint separati e applicati una sola volta anche dopo un commit fallito o un riavvio. Annullamenti, revisioni e checksum restano verificati. Il [rapporto di recupero](meetings-recupero-crash.md) include prove e comandi aggiornati.

I meeting legacy vengono convertiti solo quando necessario, mantenendo una copia `legacy.json`. Testo storico privo di tempi resta tale. Trascrizioni e sintesi hanno versioni; gli appunti modificati conservano le revisioni citate. La proiezione nello store precedente viene ricostruita dalla versione autorevole. Un blocco del sistema operativo impedisce accessi incompatibili delle due UI aggiornate; eseguibili storici anteriori a questo controllo restano esclusi dal flusso supportato.

La cancellazione scrive prima una tombstone; elimina contenuto, versioni, cache STT e audio nell'archivio, senza rimuovere il file importato esterno. Il riavvio riprende un'eventuale pulizia incompleta. I lavori ancora attivi ricevono annullamento e devono terminare prima della rimozione.

### Trascrizione e sintesi

Lo STT usa finestre di 15 secondi con 500 ms di sovrapposizione, filtro energetico del silenzio e deduplicazione. Il risultato viene pubblicato solo se completo, non annullato e riferito alla revisione ancora corrente. La cache conserva finestre concluse: il retry le riusa quando audio, modello/runtime, lingua ed endpoint coincidono. Le credenziali non entrano nella cache.

La sintesi legge tutto l'input attraverso gruppi limitati e riduce i gruppi su errori di contesto. Decisioni e attività richiedono fonti esistenti; responsabili e scadenze devono comparire nelle fonti citate. Non vengono creati automaticamente eventi, email o task. I controlli strutturali non sostituiscono una valutazione umana della correttezza semantica.

L'export mantiene la versione selezionata e i passaggi storici citati. La ricerca testuale rispetta l'esclusione del meeting dal contesto automatico; l'accesso esplicito al meeting resta disponibile. MCP conserva `list_meetings`/`read_meeting` e aggiunge `search_meetings` e `read_meeting_segments` con limite massimo di 100 passaggi.

## Verifiche ripetibili

I test automatici usano segnali, note e credenziali finti. Non aprono microfono o loopback. Le cartelle temporanee sono dentro `checks/meetings/` perché in questo ambiente Windows le sostituzioni atomiche nella cartella temporanea di sistema possono essere negate.

```powershell
# Dalla radice del progetto.
$env:QAMILLE_TEST_ROOT = Join-Path $PWD 'checks/meetings/test-data'
$env:QAMILLE_DATA = Join-Path $PWD 'checks/meetings/workspace-test'
New-Item -ItemType Directory -Force -Path $env:QAMILLE_TEST_ROOT,$env:QAMILLE_DATA | Out-Null
$env:TEMP = $env:QAMILLE_TEST_ROOT
$env:TMP = $env:QAMILLE_TEST_ROOT
cargo test --manifest-path app/Cargo.toml --no-default-features --test meetings --test meeting_storage --test meeting_mcp --test services --test layers
cargo test --manifest-path app/Cargo.toml --no-default-features --lib meeting_
npm --prefix desktop run build
npm --prefix desktop run test:meetings
# Dalla directory desktop, così viene usata la configurazione Cargo del progetto:
cd desktop
cargo build --manifest-path src-tauri/Cargo.toml --features custom-protocol
```

Il test HTTP STT contatta soltanto un server loopback creato dal test. Un ambiente che vieta socket localhost non può eseguirlo; tale restrizione va distinta da un errore del provider. Il test browser avvia un server preview privato e lo chiude al termine.

L'harness separato `checks/meetings/audio-unit/` include i sorgenti audio/storage reali e consente diagnosi isolate. Non sostituisce la compilazione del progetto completo.

**Esiti della sessione: 95 test Rust passati**: 29 unitari audio/elaborazione, 11 servizi meeting, 10 archivio, 1 integrazione MCP, 2 confini architetturali e 42 regressioni dei servizi. **9 scenari browser passati**, build web riuscita con 0 errori/avvisi Svelte. Build Windows standalone riuscita con `custom-protocol`; anche `cargo check` dell'interfaccia legacy passa, con avvisi preesistenti sui tipi numerici.

**2 controlli nell'app Windows compilata passati** con `npm --prefix desktop run test:meetings:native`: comandi reali Tauri/elenco leggero e riproduzione WAV sintetico sotto CSP di produzione; chiusura che salva insieme appunti meeting e bozza email. Il player ammette URL `blob:` nella direttiva CSP `media-src`. Il test crea e chiude soltanto la propria istanza con account Sample; non registra né scarica modelli. Il progetto conserva un avviso di futura incompatibilità della dipendenza `imap-proto`, non introdotto dai meeting.

## Differenze e prove ancora aperte

| Punto | Stato reale e prova successiva |
| --- | --- |
| Perdita dispositivo, AUD-04 | Produce `Interrupted`, conserva i blocchi e consente recupero. Non riconfigura la sorgente all'interno della stessa sessione; per continuare scegliere il dispositivo e avviare una nuova registrazione. |
| Trascrizione progressiva, TRN-02 | Disponibile soltanto dopo lo stop. La modalità live è rinviata finché il benchmark non dimostra il margine di velocità previsto dalla specifica. Nessuna diarizzazione o VAD neurale. |
| Codec, TRN-04 | Decodificatori WAV/PCM, MP3, M4A/AAC e FLAC abilitati in Symphonia. Il test automatico genera WAV; profili compressi reali e file lunghi richiedono fixture dedicate prima di dichiarare copertura completa. |
| Qualità e risorse, P0/QUA-05 | Runtime `--help` verificato; inferenza su modello locale e corpus italiano, allineamento di due dispositivi, cuffie/altoparlanti, Bluetooth e sessione di 2 ore ancora da misurare. |
| Scala e annullamento | Elenco UI paginato e senza testi completi; il dettaglio conserva versioni in JSON e la ricerca legge i documenti locali. Prestazioni su archivi molto grandi non misurate. Una richiesta AI già partita può attendere il timeout del trasporto (STT 90 s, sintesi 180 s); annullamento impedisce comunque la pubblicazione del risultato. |

La V1 non è dichiarata verificata su hardware reale. Il protocollo successivo è: 30 secondi con due sorgenti riconoscibili; pausa/ripresa e disconnessione; arresto e recupero; corpus italiano di almeno 30 minuti con riferimento umano; registrazione di 2 ore misurando allineamento, memoria e blocchi confermati. Le registrazioni reali iniziano soltanto con un'azione esplicita nell'app.

## Riferimento upstream

Meetily resta registrato alla revisione `a2cb62e827da7ef59f65064c97233efb2313878e` nel [registro upstream](../reference/README.md). L'implementazione è originale Qamille e adotta i confini studiati fra acquisizione, persistenza e AI; nessun sorgente Meetily è stato copiato. Licenze, hash e provenienza di runtime e modelli sono in [THIRD_PARTY.md](../app/src/infrastructure/meeting_processing/THIRD_PARTY.md). Una nuova release upstream resta un candidato da esaminare, senza aggiornamenti automatici del codice o dei modelli.
