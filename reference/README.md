# Repository di riferimento upstream

Questo registro conserva i repository esterni da consultare durante l'evoluzione di Qamille e la revisione di partenza per valutare i loro aggiornamenti.

## Meetily — registrazione e gestione dei meeting

**Decisione del 2 novembre 2026:** mantenere Meetily come riferimento upstream per recorder, trascrizioni, appunti e riassunti. Le modifiche upstream sono candidate da valutare e adattare all'architettura di Qamille.

| Riferimento | Valore |
| --- | --- |
| Repository / branch | [Zackriya-Solutions/meetily](https://github.com/Zackriya-Solutions/meetily) · `main` |
| Ultima revisione analizzata | [`a2cb62e827da7ef59f65064c97233efb2313878e`](https://github.com/Zackriya-Solutions/meetily/commit/a2cb62e827da7ef59f65064c97233efb2313878e) · 2 novembre 2026 |
| Aggiornamenti da esaminare | [Confronto dalla revisione analizzata a main](https://github.com/Zackriya-Solutions/meetily/compare/a2cb62e827da7ef59f65064c97233efb2313878e...main) · [Release](https://github.com/Zackriya-Solutions/meetily/releases) · [Cronologia main](https://github.com/Zackriya-Solutions/meetily/commits/main/) |
| Licenza del codice alla revisione analizzata | [MIT](https://github.com/Zackriya-Solutions/meetily/blob/a2cb62e827da7ef59f65064c97233efb2313878e/LICENSE.md); mantenere attribuzioni e verificare separatamente modelli, dipendenze e binari distribuiti |
| Stato del riuso | Architettura studiata e adattata con codice originale Qamille; nessun sorgente Meetily copiato. Pipeline disponibile, accettazione hardware aperta: [rapporto](../docs/meetings-implementazione.md). |

### Ambiti da seguire

1. **Audio:** cattura microfono e uscita del computer, selezione e cambio dispositivi; `frontend/src-tauri/src/audio/recording_manager.rs`, `audio/stream.rs`, `audio/devices/platform/`.
2. **Affidabilità:** checkpoint, recupero delle registrazioni e completamento dei lavori; `audio/incremental_saver.rs`, `audio/recording_saver.rs`, `audio/recording_commands.rs`.
3. **Trascrizione:** motori locali, italiano, segmentazione, importazione e ritrascrizione; `audio/transcription/`, `whisper_engine/`, `parakeet_engine/`, `audio/import.rs`, `audio/retranscription.rs`.
4. **Riassunti e dati:** meeting lunghi, template, lingua, timestamp e persistenza; `summary/`, `database/`, `frontend/src-tauri/migrations/`.
5. **Compatibilità e distribuzione:** versioni e licenze delle dipendenze, modelli e accelerazione; `Cargo.toml`, `Cargo.lock`, `frontend/src-tauri/Cargo.toml` e script di build.

I percorsi abbreviati sopra sono relativi a `frontend/src-tauri/src/`, salvo quelli esplicitamente indicati dalla radice. Verificare che i moduli siano collegati al flusso attivo: directory presenti e funzionalità pubblicizzate non dimostrano da sole una capacità utilizzabile.

### Procedura per una revisione upstream

1. Leggere release e confronto dalla revisione registrata; fissare il nuovo SHA da esaminare.
2. Selezionare cambiamenti pertinenti agli ambiti sopra, verificando codice attivo, licenze e compatibilità con Qamille. Distinguere il codice pubblico Community dalle funzionalità Pro.
3. Annotare per ciascun candidato l'esito: adottare, adattare, rimandare o scartare, con motivazione. Per il codice effettivamente portato, registrare commit di provenienza, file locali, attribuzioni e verifiche eseguite.
4. Aggiornare la revisione analizzata e il relativo link di confronto solo dopo avere completato la lettura delle modifiche pertinenti. Conservare nel registro lo SHA precedente e gli esiti; una revisione parziale non avanza la baseline.

La consultazione avviene quando si lavora sui meeting o si richiede un aggiornamento upstream. Un eventuale monitoraggio periodico richiede una configurazione dedicata.

### Registro delle revisioni

| Data | Revisione | Esito |
| --- | --- | --- |
| 2 novembre 2026 | `a2cb62e827da7ef59f65064c97233efb2313878e` | Prima analisi statica. Priorità: audio microfono+sistema, salvataggi intermedi, trascrizione locale e riassunti strutturati. Integrazione e prove audio da eseguire. |
| 2 novembre 2026 | Baseline invariata | Implementati in Qamille i confini acquisizione/salvataggio/STT/sintesi, checkpoint e versioni. Motore locale scelto: whisper.cpp v1.9.2 CPU in processo separato; CPAL e Symphonia del progetto. Nessun porting di sorgenti Meetily, database, UI o codice Pro. [Provenienza runtime e modelli](../app/src/infrastructure/meeting_processing/THIRD_PARTY.md). |

### Analisi conservate

- [Specifica funzionale e tecnica proposta](../docs/meetings-specifica.md).
- [Valutazione per Qamille e sequenza proposta](../docs/valutazione-meetily-qamille.md).
- [Ricerca tecnica upstream con fonti fissate al commit](../docs/ricerca-meetily-upstream.md).

Le analisi descrivono la revisione esaminata; questo registro è il punto di riferimento per gli aggiornamenti successivi.
