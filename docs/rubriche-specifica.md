# Rubriche: specifica funzionale e tecnica

**Prodotto:** Qamille desktop · **Versione:** proposta 1.0 · **Data:** 3 novembre 2026 · **Consegna:** unica.

**Obiettivo:** avere tutti i contatti di tutte le caselle e rubriche in un'unica rubrica virtuale, con i doppioni riuniti in una sola scheda che mostra da dove viene, e poter aggiungere, modificare, unire e sincronizzare senza perdere dati su nessun servizio.

Lo [studio di fattibilità](rubriche-studio.md) contiene le evidenze, le alternative scartate e i limiti di ciò che è stato verificato. Questa specifica è il riferimento dei requisiti. Le scelte marcate **[costruttore]** sono lasciate a chi realizza.

## 1. Ambito e scelte

| Voce | Scelta |
| --- | --- |
| Sorgenti | Rubrica **Locale** (portable, in `data/`), rubriche **Google** degli account Gmail (People API), file **vCard** importati (diventano Locale), **Posta** (persone viste nei messaggi, sola lettura, è la lista attuale). |
| Rubrica virtuale | Vista calcolata, mai salvata. Ogni scheda riporta le origini. |
| Duplicati | Rilevati in gruppi di 2, 3 o più schede. Segnalati; mai risolti in automatico sulle rubriche. |
| Risposte | **Stessa persona** · **Unisci…** · **Persone diverse**. |
| Contatti nuovi | Scelta obbligatoria della rubrica (Locale o un account Google con scrittura). |
| Archivio | File separato `data/contacts.json`, stesso schema di lock e scrittura atomica del calendario, dietro una porta applicativa. |
| Distribuzione | Uso personale: progetto OAuth in modalità Testing, come in [oauth-google.md](oauth-google.md). |
| Fuori ambito | CardDAV; gruppi ed etichette dei contatti; foto. I dati Google non gestiti sono conservati e riscritti invariati. |

### Stato osservato prima dell'implementazione

Nessuna rubrica. `MailService::search_suggestions` ([mail.rs](../app/src/application/mail.rs)) ricava nome e indirizzo da mittenti, A e Cc; la usano la barra di ricerca ([SearchBar.svelte](../desktop/src/components/SearchBar.svelte)) e il tool MCP `search_contacts` ([tools.rs](../app/src/adapters/mcp/tools.rs)). Il compositore ha A/Cc/Ccn come testo libero ([Composer.svelte](../desktop/src/components/Composer.svelte)). Il consenso Google del calendario è separato da quello della posta e il suo stato di sync vive in `data/calendar-state.json` ([calendar.rs](../app/src/adapters/mcp/calendar.rs)).

## 2. Esperienza principale

1. **Apri Rubrica** dalla barra laterale, sezione strumenti. La lista mostra una scheda per persona, con nome, indirizzo principale e le etichette di origine (`Google · alfio@gmail.com`, `Locale`, `Posta · lavoro@…`). Ricerca per nome, indirizzo, telefono, azienda. Filtro per origine: Tutte, oppure una rubrica.
2. **Apri una scheda:** campi, origini con stato di sincronizzazione e data dell'ultimo aggiornamento per ciascuna, pulsante Modifica.
3. **Crea un contatto:** modulo con, in cima, la scelta obbligatoria della rubrica (preselezionata: l'ultima usata; Locale la prima volta). Se nome, indirizzo o telefono coincidono con un contatto esistente, il modulo lo dice prima del salvataggio e offre di aprire quello esistente.
4. **Modifica un contatto unificato:** si cambia un campo; prima di salvare l'anteprima elenca le rubriche che verranno aggiornate. Una scheda solo "Posta" non si modifica: **Aggiungi ai contatti** apre il modulo di creazione precompilato.
5. **Elimina:** per rubrica, con conferma. Una scheda "Posta" non si elimina: si **nasconde**.
6. **Duplicati:** pannello con la lista dei gruppi, dal più certo al meno certo. Un avviso riepilogativo ("12 potenziali duplicati") compare dopo import e sincronizzazione; mai una finestra bloccante per caso.
7. **Rispondi a un gruppo** con uno dei tre pulsanti (sezione 4).
8. **Importa ed esporta** `.vcf` (vCard 3.0 e 4.0) dalla rubrica Locale.
9. **Scrivi un'email:** A, Cc, Ccn completano dalla rubrica virtuale; la barra di ricerca usa la stessa fonte.

Tutti i testi in italiano e inglese con `tr(it, en)`.

## 3. Rubrica virtuale e sovrapposizioni

### 3.1 Che cosa si salva

Si salvano solo, in `data/contacts.json`: le schede sorgente per rubrica (con id remoto, etag, ultima copia sincronizzata); i collegamenti confermati con "Stessa persona"; le coppie "Persone diverse"; i valori fissati a mano campo per campo; le schede nascoste; la coda delle scritture (sezione 5); il diario di annulla; le **impostazioni della rubrica** (ordine di priorità, indirizzi di ruolo, ultima rubrica usata, interruttori di sincronizzazione). Queste impostazioni non entrano in `Settings` di `store.json`: `Settings.svelte` le mostra tramite `ContactService`, così leggere la rubrica non prende mai il lock della posta. La vista unificata, le etichette di origine e i collegamenti automatici si ricalcolano a ogni lettura e sincronizzazione.

### 3.2 Normalizzazione (dominio, funzioni pure)

- Email: minuscolo, trim. Punti e `+tag` restano.
- Telefono: E.164 con regione dalla locale (`Settings.locale`, sottotag regione; sistema se vuota). Senza regione valida: confronto sulle ultime cifre, marcato debole.
- Nome: minuscolo, senza diacritici, punteggiatura e apostrofi; token ordinati; titoli e suffissi ignorati.

### 3.3 Regole di confronto

| Livello | Condizione | Effetto nella vista | Segnalazione |
| --- | --- | --- | --- |
| Certo | stessa email, oppure stesso telefono E.164, oppure stesso `UID` vCard | una scheda sola | sì |
| Probabile | stesso nome normalizzato e un indizio concordante (azienda, dominio email, nome+cognome) | schede separate | sì |
| Possibile | nome simile sotto soglia, senza altri indizi | schede separate | solo nel pannello |

- I gruppi si formano solo con collegamenti certi; i probabili sono coppie. Nel gruppo l'utente può togliere un membro prima di rispondere.
- Indirizzi di ruolo (`info@`, `vendite@`, `noreply@`, lista estendibile dall'utente) non producono il livello certo.
- Le origini "Posta" e "Google · altri" non provocano mai segnalazioni: una scheda "Posta" che coincide con un contatto Google o Locale viene assorbita come etichetta di origine, senza avviso. "Aggiungi ai contatti?" compare solo per le schede che esistono soltanto come "Posta".
- I livelli non compaiono nell'interfaccia: solo l'ordine della lista.
- Confronto con chiavi di raggruppamento su email, telefono e token del nome, non a coppie su tutta la rubrica.
- La soglia del livello "possibile" si fissa con il protocollo della sezione 8, non a priori.

### 3.4 Campo per campo

- Valore singolo (nome, azienda, ruolo, compleanno): vince l'origine più alta nell'ordine di priorità delle rubriche (Impostazioni); a parità, la più recente. "Posta" è sempre ultima. Un valore fissato a mano vince su tutto.
- Valori multipli (email, telefoni, indirizzi): unione senza doppioni sul valore normalizzato, etichette (casa, lavoro) conservate.
- Note: concatenate con la provenienza.

## 4. I tre pulsanti

| Pulsante | Effetto |
| --- | --- |
| **Stessa persona** | Le schede diventano una sola nella rubrica virtuale. Nessuna rubrica viene scritta. Disponibile sempre. Si annulla togliendo il collegamento. |
| **Unisci…** | Apre la scheda risultante già compilata (3.4) e modificabile, poi **Dove tenerla?**: una casella per ogni rubrica del gruppo, scorciatoie **Solo in una** e **In tutte**. Le rubriche spuntate ricevono la scheda (creata se mancava); da quelle non spuntate la copia viene eliminata. Anteprima per rubrica e conferma. |
| **Persone diverse** | Restano separate; la coppia non viene più proposta. |

Regole:
- Unisci… è l'unica operazione che elimina dati sui server.
- Una rubrica senza consenso di scrittura ha la casella disattivata con la spiegazione; il resto procede.
- Una scheda "Posta" non è una casella: non si elimina.
- **Annulla dopo una modifica successiva:** l'annulla passa dalla coda scritture con lo stesso controllo di etag; se la scheda remota è cambiata dopo l'unione, diventa un conflitto come gli altri (sezione 5), mai un ripristino cieco.
- Esito parziale: mostrato per rubrica; ciò che fallisce resta in coda e si ritenta senza ripetere le parti riuscite.
- **Annulla unione:** prima di Unisci… si salva lo stato delle schede coinvolte in un diario; l'annulla ripristina solo ciò che è stato davvero scritto. Una scheda remota eliminata viene ricreata con nuovo id: l'interfaccia lo dice. Il diario si conserva 30 giorni **[costruttore: valore modificabile]**.

## 5. Sincronizzazione Google

- **Consenso:** separato dalla posta e dal calendario, come `CALENDAR_SCOPE` in [oauth.rs](../app/src/infrastructure/oauth.rs). Lettura (`contacts.readonly`, `contacts.other.readonly`) al collegamento della rubrica. Scrittura (`contacts`): quando l'utente salva la prima modifica che tocca quella rubrica, l'app apre subito il consenso nel browser; se viene negato o annullato, la modifica **non viene salvata** e lo dice. La coda non contiene mai scritture "in attesa di consenso". Un solo segreto `contacts:{account_id}`, sostituito dal token con ambito più ampio.
- **Avvio della sincronizzazione:** all'avvio dell'app in background, all'apertura della Rubrica e con il pulsante **Aggiorna ora**; nessun job periodico. Ogni rubrica Google ha un interruttore di sincronizzazione nelle Impostazioni. Compositore e ricerca leggono subito il file locale, senza aspettare la rete.
- **Lettura:** `people.connections.list` con `requestSyncToken`, `pageSize` 1000, poi `syncToken` per account. Cancellazioni con `metadata.deleted`. Token scaduto (7 giorni, `EXPIRED_SYNC_TOKEN`): risincronizzazione completa, trattata come normale e senza avviso d'errore. "Altri contatti" letti con la stessa modalità e mostrati come origine `Google · altri · account`, trattata **come "Posta"**: sola lettura, nessuna casella in Unisci…, si nasconde invece di eliminarsi; "Aggiungi ai contatti" li promuove con `copyOtherContactToMyContactsGroup`.
- **Scrittura:** `updateContact` con `etag` e `updatePersonFields`; `createContact`; `deleteContact`. Richieste **in sequenza** per account. Etag diverso: rilettura della scheda, confronto a tre vie (base, locale, remoto) campo per campo; se entrambe le parti hanno cambiato lo stesso campo, la scheda passa "in conflitto" e l'utente sceglie **per scheda intera**, con le due versioni affiancate: **Tieni la mia** o **Tieni quella di Google**. Mai sovrascrittura cieca. La coda conserva la terna base/locale/remoto finché il conflitto non è risolto. Dopo una scrittura si applica localmente la risposta: non si rilegge via sync.
- **Coda offline:** le scritture vanno in una coda persistente nel file dei contatti, con stato per scheda: sincronizzato, in attesa, in conflitto, errore. Ritentativi con attesa crescente **[costruttore]**.
- **Dati non gestiti** (gruppi, foto, altri campi People): conservati nella copia sincronizzata e riscritti invariati.
- **Guida:** [oauth-google.md](oauth-google.md) passo 3 aggiunge People API fra le API da abilitare.

## 6. Architettura

| Livello | Elementi |
| --- | --- |
| Dominio `src/domain/contacts.rs` | `Contact`, `AddressBookId`, `ContactLink`, `Hidden`, `Rejected`, normalizzazione, regole 3.3, fusione 3.4, lettura e scrittura vCard. Nessun I/O ([layers.rs](../app/tests/layers.rs) resta verde). |
| Applicazione `src/application/contacts.rs` | `ContactService`: lista unificata, scheda, crea, modifica, elimina, nascondi, duplicati, i tre pulsanti, annulla, import/export, sincronizza. Porte in `ports.rs`: `ContactStore` (lettura, transazione atomica), `ContactGateway` (list, create, update, delete, copy_other, per account). Riuso di `SecretStore`, `Clock`, `GoogleIdentity`. |
| Infrastruttura | `google_people.rs` sul modello di `google_calendar.rs` (`ureq`, Bearer, hook di autorizzazione); `contact_repository.rs` (JSON, lock, file temporaneo, rename, come `calendar-state`); `oauth.rs`: `CONTACTS_SCOPE`. |
| Tauri | `commands/contacts.rs`, registrazione in `main.rs`, composizione in `runtime.rs`; operazioni lunghe su worker bloccanti. |
| Interfaccia | `Contacts.svelte` (lista, scheda, modulo, duplicati, Unisci…), voce in `Sidebar.svelte`, `Folder` in `types.ts`, `Settings.svelte` (rubriche, interruttori di sincronizzazione, consenso, ordine di priorità, indirizzi di ruolo, tutto via `ContactService`), `Composer.svelte` e `SearchBar.svelte` collegati alla rubrica. |
| Anteprima | `preview.ts`, `sample.json`, `backend-text.ts`. |
| MCP | `search_contacts` con `source` reale; `list_address_books`, `get_contact`, `find_duplicates` in lettura; `create_contact`, `update_contact` e `link_contacts` (= Stessa persona, nessuna scrittura remota) sotto `Access::Triage`. **Nessun tool esegue Unisci…**: l'unica operazione che elimina dati resta all'utente nell'interfaccia. `docs/mcp-tools.json` aggiornato. |

Il file `data/contacts.json` vive accanto a `store.json` nella cartella portable (`Store::data_dir`), quindi segue l'eseguibile come il resto. La sorgente "Posta" continua a derivare da `search_suggestions`.

## 7. Privacy

Nessun contatto raggiunge un provider AI senza scelta esplicita. Consenso, messaggio e revoca Google dei contatti separati da posta e calendario. Nessun segreto nei tipi serializzati verso l'interfaccia.

## 8. Verifica

- Test con repository in memoria e gateway Google finto, sul modello di [services.rs](../app/tests/services.rs).
- Dominio: corpus sintetico italiano (apostrofi, accenti, ordine invertito, omonimi, indirizzi di ruolo, `+tag`, numeri con e senza prefisso), round-trip vCard 3.0 e 4.0.
- Soglia "possibile": ipotesi "il livello probabile non propone mai un'unione errata sul corpus"; baseline uguaglianza esatta; precisione e richiamo per livello su coppie etichettate; seme fisso. Da eseguire prima di fissare la soglia.
- Sync: token scaduto, etag obsoleto, scrittura in sequenza, cancellazione remota, coda offline, esito parziale.
- Unione: i tre pulsanti, Solo in una, In tutte, annulla, ricreazione di una scheda remota eliminata.
- Interfaccia: controlli nell'anteprima browser e nell'app Tauri, come per le funzioni esistenti.
- Accesso Google reale: non verificabile in automatico; prova manuale sull'account dell'utente.

## 9. Criteri di rilascio

1. Tutti i test automatici verdi, `layers.rs` compreso.
2. Creazione, modifica, eliminazione, import ed export funzionano senza rete sulla rubrica Locale.
3. Con un account Google di prova: prima sincronizzazione, modifica locale propagata, modifica remota recepita, ripartenza dopo token scaduto, Unisci… con Solo in una e In tutte, annulla.
4. Il compositore completa gli indirizzi dalla rubrica; la barra di ricerca non regredisce.
5. Nessun dato Google non gestito viene perso dopo un ciclo lettura-scrittura (confronto prima/dopo).
