# Meetily: cosa riutilizzare in Qamille

**Data:** 2 novembre 2026. **Esito:** sì al riuso selettivo del motore audio e delle idee di gestione dei meeting. La compatibilità Rust/Tauri è favorevole, ma non equivale a un'integrazione immediata.

**Riferimento mantenuto per gli aggiornamenti:** [registro upstream di Meetily](../reference/README.md), con ultima revisione analizzata, confronto e procedura di revisione.

**Sviluppo proposto:** [specifica funzionale e tecnica](meetings-specifica.md), che dettaglia questa valutazione senza dichiarare le funzioni già realizzate.

Valutazione mediante lettura del codice: nessuna compilazione o prova audio di Meetily eseguita. Nessuna funzionalità applicativa modificata. Riferimento upstream: commit [`a2cb62e827da7ef59f65064c97233efb2313878e`](https://github.com/Zackriya-Solutions/meetily/tree/a2cb62e827da7ef59f65064c97233efb2313878e). Per l'inventario tecnico upstream e le licenze dei componenti, vedere [ricerca-meetily-upstream.md](ricerca-meetily-upstream.md).

## 1. Punto di partenza verificato nel nostro codice

| Area | Stato attuale di Qamille | Conseguenza |
| --- | --- | --- |
| Registrazione | [recorder.rs](../app/src/infrastructure/recorder.rs) registra il microfono predefinito con CPAL e salva WAV mono tramite Hound; pausa/ripresa e livello sono presenti. | Non acquisisce direttamente l'audio riprodotto da Teams/Meet/Zoom. Con cuffie la voce degli altri non entra nel microfono. |
| Trascrizione | [ai.rs](../app/src/infrastructure/ai.rs), `transcribe`, carica l'intero WAV e lo invia a un endpoint compatibile OpenAI, con modello `whisper-1`. | Manca un motore locale integrato e manca la trascrizione progressiva. Il comportamento dell'endpoint dipende dal provider configurato. |
| Riassunti | `summarize_meeting` nello stesso file invia trascrizione e appunti in una sola richiesta; estrae riassunto, punti chiave e azioni da JSON. Se fallisce usa regole estrattive. | Per meeting lunghi servono gestione della dimensione, errori espliciti e risultati verificabili. Ollama è già supportato per la generazione del testo. |
| Interfaccia desktop | [Meetings.svelte](../desktop/src/components/Meetings.svelte) elenca meeting, mostra sintesi/trascrizione e salva appunti. I comandi di registrazione e trascrizione sono ancora in [ui.rs](../app/src/adapters/native/ui.rs). | Portare il flusso completo nella nuova UI è una parte necessaria del lavoro. |
| Dati e ricerca | [Meeting](../app/src/domain/model.rs) contiene una trascrizione come stringa, senza segmenti temporali o identificatori del parlante. [assistant.rs](../app/src/application/assistant.rs) aggiunge al contesto i riassunti dei meeting. | Non abbiamo ancora ricerca semantica dei passaggi dei meeting con citazione dell'istante audio. |

## 2. Le cinque opportunità, ordinate per utilità

### 1. Registrare microfono e audio del computer

È il primo investimento: acquisire entrambi i lati della conversazione, scegliere dispositivi e mostrare separatamente i livelli. L'implementazione attiva di Meetily offre un riferimento concreto per Windows attraverso [`recording_manager.rs`](https://github.com/Zackriya-Solutions/meetily/blob/a2cb62e827da7ef59f65064c97233efb2313878e/frontend/src-tauri/src/audio/recording_manager.rs), [`stream.rs`](https://github.com/Zackriya-Solutions/meetily/blob/a2cb62e827da7ef59f65064c97233efb2313878e/frontend/src-tauri/src/audio/stream.rs) e [`devices/platform/windows.rs`](https://github.com/Zackriya-Solutions/meetily/blob/a2cb62e827da7ef59f65064c97233efb2313878e/frontend/src-tauri/src/audio/devices/platform/windows.rs).

Adattamento proposto: un servizio di registrazione nel nostro motore condiviso, esposto tramite comandi Tauri. La UI resta Svelte. Conservare, se sostenibile, le sorgenti separate oltre al mix: aiuta la diagnosi e le elaborazioni future, ma non identifica automaticamente i singoli partecipanti remoti.

### 2. Salvare durante la registrazione e recuperare dopo un'interruzione

Meetily ha [`IncrementalAudioSaver`](https://github.com/Zackriya-Solutions/meetily/blob/a2cb62e827da7ef59f65064c97233efb2313878e/frontend/src-tauri/src/audio/incremental_saver.rs), con checkpoint a soglia di circa 30 secondi e ricomposizione finale. È un modello utile; non dimostra da solo assenza di perdita audio. La sua implementazione usa audio misto mono e FFmpeg.

Proposta per noi: persistere sessione e blocchi audio prima della trascrizione, mostrare lo stato di recupero e permettere di riprovare l'elaborazione. Valutare blocchi WAV con Hound, già presente, prima di introdurre il confezionamento e gli obblighi di una distribuzione FFmpeg.

### 3. Trascrizione locale, segmenti temporali e importazione

Il [manifest Meetily](https://github.com/Zackriya-Solutions/meetily/blob/a2cb62e827da7ef59f65064c97233efb2313878e/frontend/src-tauri/Cargo.toml) identifica componenti concreti: `whisper-rs`, Silero per rilevare la voce, `rubato` per ricampionare e `symphonia` per decodificare file audio. È possibile usarli direttamente, anche senza inglobare l'intera applicazione. La scelta del modello italiano e la compatibilità delle dipendenze vanno verificate nel prototipo.

Per il confronto italiano, candidati sono Whisper multilingue e [Parakeet TDT 0.6B v3](https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3), che include l'italiano nelle 25 lingue dichiarate. [Parakeet v2](https://huggingface.co/nvidia/parakeet-tdt-0.6b-v2) è invece per l'inglese. I modelli NVIDIA hanno licenza CC BY 4.0: non ereditano la MIT del codice Meetily. Il supporto linguistico dichiarato non misura la qualità sul nostro audio reale.

Il [modello Transcript upstream](https://github.com/Zackriya-Solutions/meetily/blob/a2cb62e827da7ef59f65064c97233efb2313878e/frontend/src-tauri/src/database/models.rs) conserva inizio, fine e durata dei segmenti. Adotterei lo stesso principio: clic sul testo per ascoltare il passaggio, importazione di registrazioni precedenti e nuova trascrizione con lingua/modello diverso. La trascrizione esistente va conservata quando una nuova elaborazione fallisce.

### 4. Riassunti adatti a riunioni lunghe

Il [`processor.rs`](https://github.com/Zackriya-Solutions/meetily/blob/a2cb62e827da7ef59f65064c97233efb2313878e/frontend/src-tauri/src/summary/processor.rs) contiene divisione del testo, sintesi intermedie, ricomposizione, annullamento e tentativi limitati. Il ramo di suddivisione è attivato per Ollama/BuiltInAI, non indistintamente per tutti i provider. Il codice genera una base inglese prima della traduzione: per noi è preferibile valutare generazione diretta in italiano, usando il modello scelto dall'utente.

Sono già presenti nel codice pubblico [modelli di report e relative istruzioni](https://github.com/Zackriya-Solutions/meetily/blob/a2cb62e827da7ef59f65064c97233efb2313878e/frontend/src-tauri/src/summary/templates/defaults.rs). Adatterei lo schema a sintesi, decisioni, attività, responsabili e scadenze. Responsabili/scadenze non detti devono rimanere vuoti. Collegare ogni decisione al segmento sorgente è una nostra proposta aggiuntiva, non una funzionalità upstream verificata.

### 5. Unire meeting, calendario, appunti e ricerca

Qamille ha già `event_id`, appunti, provider AI e strumenti MCP per leggere meeting. Conviene estendere questi punti di integrazione. Proposte: appunti con riferimento temporale, ricerca dentro le trascrizioni, risposte con citazioni ascoltabili, esportazione e trasformazione esplicita di un'azione in attività o bozza email.

Queste funzioni sono una direzione per Qamille, non una promessa che siano tutte estraibili da Meetily. L'[architettura upstream](https://github.com/Zackriya-Solutions/meetily/blob/a2cb62e827da7ef59f65064c97233efb2313878e/docs/architecture.md) descrive un'app desktop Tauri con SQLite; non documenta in quella pagina un servizio SaaS o SDK pubblico pronto da collegare.

## 3. Riuso consentito e limiti concreti

1. **Codice MIT:** la [licenza del repository](https://github.com/Zackriya-Solutions/meetily/blob/a2cb62e827da7ef59f65064c97233efb2313878e/LICENSE.md) consente uso, copia, modifica e vendita, mantenendo avviso di copyright e licenza nelle copie o porzioni sostanziali. La licenza MIT non impone di pubblicare tutto Qamille. Componenti, modelli, binari distribuiti e marchi richiedono valutazione separata.
2. **Community e Pro:** il [README](https://github.com/Zackriya-Solutions/meetily/blob/a2cb62e827da7ef59f65064c97233efb2313878e/README.md) descrive Pro come codice separato. Non assumere che riconoscimento dei parlanti, funzioni enterprise o tutte le esportazioni pubblicizzate siano nel repository MIT. Per i template il codice pubblico è più informativo della tabella commerciale.
3. **Dipendenze da adattare:** noi usiamo CPAL 0.16 e ORT esatto `2.0.0-rc.15`; il manifest upstream dichiara CPAL 0.15.3, una patch CPAL e ORT `2.0.0-rc.10`. Il [lockfile della workspace](https://github.com/Zackriya-Solutions/meetily/blob/a2cb62e827da7ef59f65064c97233efb2313878e/Cargo.lock) risolve CPAL dal registry: la patch nel manifest membro non va interpretata automaticamente come fork effettivamente usato. L'integrazione può richiedere porting o isolamento del motore; non aggiungere il loro manifest integralmente.
4. **Elaborazione locale:** registrazioni e trascrizioni possono restare sul computer usando modelli locali. Se si sceglie un provider remoto per i riassunti, il testo necessario viene trasmesso a quel provider. L'etichetta generale “local-first” non sostituisce questa distinzione.

## 4. Sequenza consigliata

1. **Prototipo Windows:** registrazione microfono + uscita audio nella nuova UI, livelli separati, pausa/ripresa, salvataggi intermedi. Stima preliminare: 1–2 giornate di sviluppo per dimostrare la fattibilità, non per una versione pronta alla distribuzione.
2. **Trascrizione:** motore locale multilingue, importazione file, segmenti con timestamp e ripetizione dei lavori falliti; mantenere l'endpoint remoto come opzione.
3. **Gestione meeting:** sintesi italiane strutturate, appunti collegati ai passaggi, ricerca e collegamento al calendario; in seguito identificazione dei parlanti, se supportata da una pipeline verificata.

Il primo prototipo deve dimostrare acquisizione di entrambe le voci con cuffie, pausa/ripresa senza disallineamento e recupero dei blocchi salvati dopo un arresto. Prima di stabilizzare: provare cambio dispositivo, audio assente, disco pieno e una registrazione lunga. Per la trascrizione confrontare italiano reale, nomi propri e termini tecnici sullo stesso audio; per le sintesi controllare attività e scadenze contro i passaggi originali. Questi sono criteri futuri di accettazione, non test già eseguiti.
