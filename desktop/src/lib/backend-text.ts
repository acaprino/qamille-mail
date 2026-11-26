// Italian display text for messages generated in the Rust backend, which reach the
// UI as plain English strings. Keys mirror the Rust literals exactly, so a reworded
// Rust message falls back to English instead of showing a wrong translation.
// Search query errors live in searchErrorMessage (search.ts) and are not repeated here.

type Replacement = string | ((...captures: string[]) => string);

// Longer lines are mail content, never a backend message.
const MAX_LINE_LENGTH = 4000;
const MAX_JOINED_PARTS = 16;

const FOLDERS: Record<string, string> = {
  Inbox: 'Posta in arrivo',
  Sent: 'Posta inviata',
  Archive: 'Archivio',
  Drafts: 'Bozze',
  Trash: 'Cestino',
  Spam: 'Spam',
};

const EXACT = new Map<string, string>([
  ['Local search index is being rebuilt', 'Ricostruzione dell’indice di ricerca in corso.'],
  ['Local search index needs rebuilding; the original mail is unchanged', 'L’indice di ricerca deve essere ricostruito. Le email originali sono conservate.'],
  ['Audio contains missing intervals; see the recording timeline', 'La registrazione contiene intervalli audio mancanti; verifica la timeline.'],
  ['Some audio blocks have no integrity receipt; audio is kept but its integrity cannot be verified', "Alcuni blocchi non hanno una ricevuta di integrità. L'audio è conservato, ma l'integrità non può essere verificata."],
  // Records looked up by id (AppError::missing)
  ['Account not found', 'Account non trovato.'],
  ['Attachment not found', 'Allegato non trovato.'],
  ['Attività not found', 'Attività non trovata.'],
  ['Lavoro del risultato recuperato not found', 'Lavoro del risultato recuperato non trovato.'],
  ['Draft not found', 'Bozza non trovata.'],
  ['Event not found', 'Evento non trovato.'],
  ['Evento not found', 'Evento non trovato.'],
  ['Lavoro not found', 'Lavoro non trovato.'],
  ['Meeting not found', 'Meeting non trovato.'],
  ['Message not found', 'Messaggio non trovato.'],
  ['Passaggio not found', 'Passaggio non trovato.'],
  ['Recording not found', 'Registrazione non trovata.'],
  ['Selected message not found', 'Messaggio selezionato non trovato.'],
  ['Signature not found', 'Firma non trovata.'],
  ['Trascrizione not found', 'Trascrizione non trovata.'],
  ['Undo action not found', 'Azione da annullare non trovata.'],
  ['Verbale not found', 'Verbale non trovato.'],
  ['Versione sintesi not found', 'Versione sintesi non trovata.'],
  ['Versione trascrizione not found', 'Versione trascrizione non trovata.'],

  // Accounts and sign-in
  ['Save your Google client ID in Settings first', 'Salva prima il tuo client ID Google nelle Impostazioni.'],
  ['Google sign-in is not configured in this version of Qamille. Contact the app developer.', 'L’accesso Google non è ancora configurato in questa versione di Qamille. Apri la guida per attivarlo.'],
  ['Google sign-in is already in progress. Complete it in your browser.', 'L’accesso Google è già in corso. Completalo nel browser.'],
  ["Google couldn't verify this version of Qamille. Contact the app developer.", 'Google non ha riconosciuto questa configurazione di Qamille. Controlla ID client e secret nelle opzioni sviluppatore.'],
  ['Enter an email address and valid sign-in details', 'Inserisci un indirizzo email e dati di accesso validi.'],
  ['Reconnect this mail account in Settings', 'Riconnetti questo account di posta nelle Impostazioni.'],
  ["Google's reply couldn't be read.", 'Impossibile leggere la risposta di Google.'],
  ["The reply didn't match this sign-in, so it was ignored. Please try again.", 'La risposta non corrispondeva a questo accesso ed è stata ignorata. Riprova.'],
  ["Sign-in was cancelled in the browser, or this address isn't a test user of your Google project.", "L'accesso è stato annullato nel browser, oppure questo indirizzo non è un utente di test del tuo progetto Google."],
  ["Google's reply had no sign-in code.", 'La risposta di Google non conteneva un codice di accesso.'],
  ['Google no longer accepts this sign-in. Please sign in with Google again. With a project in Testing mode this happens every 7 days.', 'Google non accetta più questo accesso. Accedi di nuovo con Google. Con un progetto in modalità Testing succede ogni 7 giorni.'],
  ["Google doesn't recognise this client ID and secret. Check them in Settings.", 'Google non riconosce questo client ID e questo secret. Controllali nelle Impostazioni.'],
  ["Google didn't share the account's address.", "Google non ha condiviso l'indirizzo dell'account."],
  ['Sign-in stopped.', 'Accesso interrotto.'],
  ['No answer from the browser after 5 minutes. Please try again.', 'Nessuna risposta dal browser dopo 5 minuti. Riprova.'],
  ["Google didn't grant lasting access. Please try again.", 'Google non ha concesso un accesso duraturo. Riprova.'],
  ['Please sign in with Google again.', 'Accedi di nuovo con Google.'],
  ["Google didn't answer the sending check. Try again in a minute.", 'Google non ha risposto al controllo di invio. Riprova tra un minuto.'],
  ["Google refused this address and password. Use an app password from Google's page, not your normal password; it needs 2-Step Verification turned on.", 'Google ha rifiutato questo indirizzo e questa password. Usa una password per le app dalla pagina di Google, non la password normale: richiede la verifica in due passaggi attiva.'],
  ['Google refused the Qamille sign-in. Sign in with Google again and tick the Gmail box on the permissions page.', "Google ha rifiutato l'accesso di Qamille. Accedi di nuovo con Google e spunta la casella Gmail nella pagina dei permessi."],
  ["You didn't give Qamille access to Gmail. Try again and tick the Gmail box on Google's permissions page.", "Non hai dato a Qamille l'accesso a Gmail. Riprova e spunta la casella Gmail nella pagina dei permessi di Google."],
  ["Google didn't grant every permission Qamille asked for. Try again and tick all the boxes on Google's permissions page.", 'Google non ha concesso tutti i permessi richiesti da Qamille. Riprova e spunta tutte le caselle nella pagina dei permessi di Google.'],
  ['Google wants you to confirm this sign-in in your browser. Open Gmail on the web once, then try again.', 'Google chiede di confermare questo accesso nel browser. Apri Gmail sul web una volta, poi riprova.'],
  ['Google is limiting this account for now. Wait a few minutes and try again.', 'Google sta limitando questo account per il momento. Attendi qualche minuto e riprova.'],
  ['IMAP access is switched off for this account. On a work account, your administrator can turn it on.', "L'accesso IMAP è disattivato per questo account. Su un account di lavoro può attivarlo l'amministratore."],
  ["Couldn't reach Google. Check the internet connection and any firewall or antivirus.", "Impossibile raggiungere Google. Controlla la connessione a Internet e l'eventuale firewall o antivirus."],
  ['Sign-in failed.', 'Accesso non riuscito.'],

  // Drafts and sending
  ['Add an account first', 'Aggiungi prima un account.'],
  ['Draft operation lock failed', "Impossibile bloccare l'operazione sulla bozza."],
  ['Undo the pending action before editing this draft', "Annulla l'azione in sospeso prima di modificare questa bozza."],
  ['Scheduled messages must be within 180 days', 'I messaggi programmati devono rientrare nei prossimi 180 giorni.'],
  ['This draft changed elsewhere. Reopen it before saving.', 'Questa bozza è stata modificata altrove. Riaprila prima di salvare.'],
  ['This draft is linked to a cloud account or is synchronizing. Create a new draft to send from another account.', 'Questa bozza è collegata a un account cloud o è in sincronizzazione. Crea una nuova bozza per inviare da un altro account.'],
  ['The cloud draft connection changed during synchronization. Reopen the draft before retrying.', 'Il collegamento della bozza cloud è cambiato durante la sincronizzazione. Riapri la bozza prima di riprovare.'],
  ['Delivery is uncertain. Check the provider before sending again.', 'L’esito dell’invio è incerto. Controlla la posta del provider prima di inviare di nuovo.'],
  ['Message was sent but its local copy could not be saved. Do not send it again.', 'Il messaggio è stato inviato, ma la copia locale non è stata salvata. Non inviarlo di nuovo.'],
  ['Delivery has not completed. Check the outbox before sending again.', 'L’invio non è completato. Controlla la posta in uscita prima di inviare di nuovo.'],
  ['This provider does not support cloud drafts', 'Questo provider non supporta le bozze nel cloud.'],
  ['Provider label not found', 'Etichetta del provider non trovata.'],
  ['Provider label management is unavailable', 'La gestione delle etichette del provider non è disponibile.'],
  ['Operation not found', 'Operazione non trovata.'],
  ['Cancel the queued delivery before editing this draft', "Annulla l'invio in coda prima di modificare questa bozza."],
  ['An outgoing attachment is unavailable', 'Un allegato in uscita non è disponibile.'],
  ['Invalid attachment name', "Nome dell'allegato non valido."],
  ['Draft changed while preparing its attachments', 'La bozza è cambiata durante la preparazione degli allegati.'],
  ['The key was already used with different content', 'La chiave di invio è già stata usata con un contenuto diverso.'],
  ['This draft already has an active or uncertain delivery. Check the outbox.', "Questa bozza ha già un invio attivo o dall'esito incerto. Controlla la posta in uscita."],
  ['Send permission was revoked', "L'autorizzazione all'invio è stata revocata."],
  ['Scheduled delivery is over 15 minutes late; review it explicitly', "L'invio programmato è in ritardo di oltre 15 minuti; controllalo prima di procedere."],
  ['Delivery claim expired; verify the provider before sending again', "La presa in carico dell'invio è scaduta; verifica sul provider prima di inviare di nuovo."],

  // MCP settings
  ['Existing Qamille profile permissions are preserved. Installation does not grant mail, calendar or AI access.', 'L’installazione mantiene i permessi del profilo Qamille scelto.'],
  ['Preparation is local; finish installation in the assistant app.', 'Completa l’importazione nell’app dell’assistente.'],
  ['Start a new Codex session and check that the Qamille tools are available.', 'Apri una nuova sessione Codex e verifica che gli strumenti Qamille siano disponibili.'],
  ['Start a new Claude Code session and check /mcp and /skills.', 'Apri una nuova sessione Claude Code e verifica /mcp e la skill Qamille.'],
  ['Quit and reopen Claude Desktop to load the MCP connection.', 'Chiudi completamente e riapri Claude Desktop per caricare il collegamento MCP.'],
  ['Restart ChatGPT desktop, open Plugins > Personal, find Qamille and complete installation. Use a new local Work chat.', 'Riavvia ChatGPT desktop, apri Plugin → Personali e installa Qamille. Inizia una nuova chat Work sul computer.'],
  ["This prepares a personal local plugin. It does not publish to the public directory or enable the plugin automatically. ChatGPT web and custom GPTs do not read this PC's local configuration.", 'Il plugin personale è pronto per ChatGPT desktop. Per ChatGPT sul web e i GPT personalizzati serve un collegamento remoto.'],
  ['Open the .mcpb extension in Claude Desktop and complete its installation; enable the connector in Cowork where available.', 'Apri l’estensione .mcpb in Claude Desktop, completa l’installazione e abilita il collegamento in Cowork.'],
  ['In Claude Customize > Add > Upload Skill, upload the skill ZIP; this does not install an MCP connection.', 'In Claude apri Personalizza → Aggiungi → Carica skill e seleziona lo ZIP della skill.'],
  ['In Claude Customize > Add > Upload Skill, upload the skill ZIP and enable it for Cowork.', 'In Claude apri Personalizza → Aggiungi → Carica skill, seleziona lo ZIP e abilita la skill per Cowork.'],
  ["The extension and skill are prepared locally; complete their installation in Claude. The extension points to this PC's Qamille data directory.", 'Estensione e skill sono pronte da importare in Claude. Il collegamento usa i dati Qamille di questo PC.'],
  ['Save this MCP profile in Qamille before connecting an assistant', 'Salva prima il profilo MCP nelle impostazioni Qamille.'],
  ['Save the selected assistant profile in Qamille Settings first', 'Salva prima il profilo scelto nelle impostazioni Qamille.'],
  ['Repair the assistant profiles in Qamille Settings before installing', 'Correggi i profili degli assistenti nelle impostazioni Qamille prima di installare.'],
  ['The executable, profile or client files changed after the preview. Review a fresh preview before installing.', 'I file o il profilo sono cambiati dopo l’anteprima. Apri una nuova anteprima prima di installare.'],
  ['A different or locally modified Qamille integration already exists. It was preserved; choose another client or resolve that entry before installing.', 'Esiste già un’integrazione Qamille diversa o modificata. È stata conservata: controllala prima di installare.'],
  ['Select the MCP server, the skill, or both', 'Seleziona il collegamento MCP, la skill o entrambi.'],
  ['Choose a valid saved assistant profile', 'Scegli un nome di profilo valido.'],
  ['Codex configuration contains invalid TOML; original file preserved', 'La configurazione Codex non è valida. Il file originale è stato conservato.'],
  ['Assistant configuration contains invalid JSON; original file preserved', 'La configurazione dell’assistente non è valida. Il file originale è stato conservato.'],
  ['The personal plugin catalog is invalid JSON; original file preserved', 'Il catalogo personale dei plugin non è valido. Il file originale è stato conservato.'],
  ['Prepared installation not found', 'Prepara prima l’installazione per questo assistente.'],
  ['MCP permissions file is invalid; repair it in Settings', 'Il file dei permessi MCP non è valido; correggilo nelle Impostazioni.'],
  ['Profile names use letters, digits, underscore or hyphen', 'I nomi dei profili possono contenere lettere, cifre, trattini bassi o trattini.'],
  ['Too many account grants', 'Troppi account autorizzati.'],
  ['Windows startup folder is unavailable', 'La cartella di avvio di Windows non è disponibile.'],
  ['Automatic startup is configured by the operating system on this platform', "Su questa piattaforma l'avvio automatico è configurato dal sistema operativo."],
  ['Add at least one recipient', 'Aggiungi almeno un destinatario.'],
  ['Check the recipient addresses', 'Controlla gli indirizzi dei destinatari.'],
  ['Attachments exceed the 25 MB limit', 'Gli allegati superano il limite di 25 MB.'],
  ['An attachment is no longer available. Remove it or attach it again.', 'Un allegato non è più disponibile. Rimuovilo o allegalo di nuovo.'],
  ['Attachments must be files', 'Gli allegati devono essere file.'],
  ['Missing attachment data', "Dati dell'allegato mancanti."],
  ['Invalid message date', 'Data del messaggio non valida.'],

  // Contacts
  ['Choose an address book', 'Scegli una rubrica.'],
  ['This address book cannot receive new contacts', 'Questa rubrica non può ricevere nuovi contatti.'],
  ['This address book cannot receive contacts', 'Questa rubrica non può ricevere contatti.'],
  ['Add a name, an email address or a phone number', 'Aggiungi un nome, un indirizzo email o un numero di telefono.'],
  ['This contact is only seen in mail. Add it to an address book first', 'Questo contatto compare solo nella posta. Aggiungilo prima a una rubrica.'],
  ['Cards seen in mail cannot be deleted; hide them instead', 'Le schede viste nella posta non si eliminano: nascondile.'],
  ['The contact is not in that address book', 'Il contatto non è in quella rubrica.'],
  ['Contact not found', 'Contatto non trovato.'],
  ['Merge history not found', "Cronologia dell'unione non trovata."],
  ['Select at least two cards', 'Seleziona almeno due schede.'],
  ['Choose at least one address book', 'Scegli almeno una rubrica.'],
  ['Write permission not granted for this address book', 'Permesso di scrittura non concesso per questa rubrica.'],
  ['Contacts data is damaged; original file preserved', 'I dati dei contatti sono danneggiati; il file originale è stato conservato.'],
  ['Remote contacts are unavailable', 'I contatti remoti non sono disponibili.'],
  ['This address book is not synchronized with Google', 'Questa rubrica non è sincronizzata con Google.'],
  ['This address book belongs to an account that is not connected to Google', 'Questa rubrica appartiene a un account non collegato a Google.'],
  ['Only Google address books need consent', 'Solo le rubriche Google richiedono il consenso.'],
  ['This card has no conflict to resolve', 'Questa scheda non ha conflitti da risolvere.'],
  ["This contact is not one of Google's other contacts", 'Questo contatto non fa parte degli «Altri contatti» di Google.'],
  ['Only the local and Google address books can be exported', 'Si possono esportare solo la rubrica locale e quelle Google.'],
  ['Allow access to Google Contacts for this account first', "Consenti prima l'accesso ai contatti Google per questo account."],
  ['Conflict: choose which version to keep', 'Conflitto: scegli quale versione tenere.'],
  ['Unexpected contact identifier from Google', 'Identificativo di contatto inatteso da Google.'],
  ['The Google sign-in used a different email; no credentials were changed', "L'accesso Google ha usato un'altra email; nessuna credenziale è stata modificata."],
  ['The contact kept changing on the server; try again', 'Il contatto continua a cambiare sul server; riprova.'],

  // Mail service and search
  ['Available online · Open to download', 'Disponibile online · Apri per scaricare'],
  ['In message text', 'Nel testo'],
  ['Matches the filters', 'Corrisponde ai filtri'],
  ['Local results: some mail on the server may be missing. Search the server to complete this search.', 'Risultati locali: parte della posta sul server potrebbe mancare. Cerca sul server per completare questa ricerca.'],
  ['Partial results: some message or attachment text is unavailable.', 'Risultati parziali: alcuni messaggi o allegati non hanno testo ricercabile disponibile.'],
  ['Server checked for this search. Results reflect mail available at the time of the check.', 'Server controllato per questa ricerca. I risultati descrivono la posta disponibile al momento del controllo.'],
  ['Fuso non riconosciuto: uso UTC / Unknown time zone: using UTC', 'Fuso non riconosciuto: uso UTC.'],
  ['A server search is already running', 'Una ricerca sul server è già in corso.'],
  ['Search state unavailable', 'Stato della ricerca non disponibile.'],
  ['Server search cancelled', 'Ricerca sul server annullata.'],
  ['Server search is unavailable for this account', 'La ricerca sul server non è disponibile per questo account.'],
  ['Search is too long (maximum 8192 bytes)', 'La ricerca è troppo lunga (massimo 8192 byte).'],
  ['Enter a file extension or MIME type', "Inserisci un'estensione di file o un tipo MIME."],
  ['Enter a duration, for example 7d', 'Inserisci una durata, per esempio 7d.'],
  ['Sync this conversation before printing the complete thread', 'Sincronizza questa conversazione prima di stamparla per intero.'],
  ['Use MBOX to download more than one message', 'Usa MBOX per scaricare più di un messaggio.'],
  ['Some original messages could not be downloaded. No export was saved.', 'Non è stato possibile scaricare alcuni messaggi originali. Nessuna esportazione è stata salvata.'],
  // Attached messages (.eml) and messages dropped on a draft
  ['Select at least one message', 'Seleziona almeno un messaggio.'],
  ['The attached message could not be read.', 'Non è stato possibile leggere il messaggio allegato.'],
  ['The attachment is not an email message.', "L'allegato non è un messaggio email."],
  ['The attachment is nested too deeply to open.', "L'allegato è annidato troppo in profondità per aprirlo."],
  ['The attachment is no longer available.', "L'allegato non è più disponibile."],
  ['The document reader is unavailable.', 'Il lettore dei documenti non è disponibile.'],
  ['The attachment text is too long to show.', "Il testo dell'allegato è troppo lungo da mostrare."],
  ['Some messages could not be downloaded. Nothing was attached.', 'Non è stato possibile scaricare alcuni messaggi. Non è stato allegato nulla.'],
  ['The messages are too large to attach: attachments are limited to 25 MB', 'I messaggi sono troppo grandi da allegare: gli allegati sono limitati a 25 MB.'],
  ['Attach at most 50 messages at a time', 'Allega al massimo 50 messaggi alla volta.'],
  ['No mail action to apply', 'Nessuna azione da applicare alla posta.'],
  ['Mail operation lock failed', "Impossibile bloccare l'operazione sulla posta."],
  ['Account storage settings changed during sync. Sync again to apply the new settings.', "Le impostazioni di archiviazione dell'account sono cambiate durante la sincronizzazione. Sincronizza di nuovo per applicare le nuove impostazioni."],
  ['This provider has not verified a complete offline copy of the account', "Questo provider non ha verificato una copia offline completa dell'account."],
  ['This message is online only. Connect to Gmail to download it.', 'Questo messaggio è disponibile solo online. Connettiti a Gmail per scaricarlo.'],
  ['Sync this message before applying a server action', "Sincronizza questo messaggio prima di applicare un'azione sul server."],
  ['Mail notifications are unavailable; periodic sync remains active', 'Le notifiche email non sono disponibili; la sincronizzazione periodica resta attiva.'],
  ['Original message download is unavailable', 'Il download del messaggio originale non è disponibile.'],
  ['The complete conversation could not be downloaded', 'Non è stato possibile scaricare la conversazione completa.'],
  ['Unrecognised date', 'Data non riconosciuta.'],
  ['Unrecognised date or preset', 'Data o preset non riconosciuti.'],
  ['Unsupported date action', 'Azione sulla data non supportata.'],
  ['Date unavailable', 'Data non disponibile'],
  ['Date out of range', 'Data fuori intervallo.'],
  ['Check the complete date and time (YYYY-MM-DDTHH:mm)', 'Controlla data e ora complete (AAAA-MM-GGTHH:mm).'],

  // Local storage
  ['Enter a positive storage limit, or choose Unlimited', 'Inserisci un limite di archiviazione positivo oppure scegli Illimitato.'],
  ['This account cannot fit in that limit without removing protected data. Choose a larger limit or Unlimited.', 'Questo account non rientra nel limite senza rimuovere dati protetti. Scegli un limite più alto o Illimitato.'],
  ['Choose a positive account storage limit or Unlimited.', "Scegli un limite di archiviazione positivo per l'account oppure Illimitato."],
  ['Disk reservation unavailable', 'Prenotazione dello spazio su disco non disponibile.'],
  ['Not enough disk space for this operation', 'Spazio su disco insufficiente per questa operazione.'],
  ['Not enough free disk space. The operation is paused; no mail was removed.', "Spazio libero su disco insufficiente. L'operazione è in pausa; nessuna email è stata rimossa."],
  ['Storage directory is unavailable', 'La cartella di archiviazione non è disponibile.'],
  ['Repository invoked a transaction twice', "Errore interno dell'archivio: transazione eseguita due volte."],
  ['Repository did not execute the transaction', "Errore interno dell'archivio: transazione non eseguita."],
  ['Repository lock failed', "Impossibile bloccare l'archivio."],

  // Undo and settings
  ['Choose a valid date', 'Scegli una data valida.'],
  ['Choose a send time', 'Scegli un orario di invio.'],
  ['Choose a send time at least 30 seconds ahead, within 180 days', 'Scegli un orario di invio tra almeno 30 secondi ed entro 180 giorni.'],
  ['Unknown draft action', 'Azione sulla bozza sconosciuta.'],
  ['This draft already has a pending action. Undo it first.', "Questa bozza ha già un'azione in sospeso. Annullala prima."],
  ['The undo time has expired', 'Il tempo per annullare è scaduto.'],
  ['The app closed before this action was confirmed. Check the mailbox before trying again.', "L'app si è chiusa prima della conferma di questa azione. Controlla la casella di posta prima di riprovare."],
  ['Choose a recognized IANA time zone', 'Scegli un fuso orario IANA riconosciuto.'],
  ['Choose a valid regional format', 'Scegli un formato regionale valido.'],
  ['Undo time must be between 5 and 30 seconds', 'Il tempo per annullare deve essere compreso tra 5 e 30 secondi.'],
  ['Choose a meeting reminder time of 1, 2, 5 or 10 minutes', 'Scegli un anticipo del promemoria di 1, 2, 5 o 10 minuti.'],
  ['No audio device is available', 'Nessun dispositivo audio disponibile.'],
  ['This meeting reminder is no longer available', 'Questo promemoria non è più disponibile.'],
  ['This window cannot open links', 'Questa finestra non può aprire link.'],
  ['Enter your name', 'Inserisci il tuo nome.'],
  ['Enter a title and an end time after the start', "Inserisci un titolo e un orario di fine successivo all'inizio."],
  ['Meeting links must start with https://', 'I link dei meeting devono iniziare con https://'],
  ['Enter a signature name', 'Inserisci un nome per la firma.'],
  ['This link type is not supported', 'Questo tipo di link non è supportato.'],
  ['The operation stopped unexpectedly. Retry the operation.', "L'operazione si è interrotta inaspettatamente. Riprova."],

  // Calendar
  ['Use an RFC3339 date-time with offset, or YYYY-MM-DD for a date', 'Usa una data e ora RFC3339 con offset, oppure AAAA-MM-GG per una data.'],
  ['All-day event boundaries must use YYYY-MM-DD; end is exclusive', 'I limiti di un evento di un giorno intero devono usare AAAA-MM-GG; la fine è esclusa.'],
  ['Timed events require RFC3339 with an explicit offset', 'Gli eventi con orario richiedono RFC3339 con un offset esplicito.'],
  ['Calendar event has no date', "L'evento del calendario non ha una data."],
  ['Calendar date is out of range', 'La data del calendario è fuori intervallo.'],
  ['Choose an increasing range and a duration from 1 to 1440 minutes', 'Scegli un intervallo crescente e una durata da 1 a 1440 minuti.'],
  ['Event ID missing', 'ID evento mancante.'],
  ['Unsupported calendar HTTP method', 'Metodo HTTP del calendario non supportato.'],
  ['Calendar provider returned an unreadable response', 'Il provider del calendario ha restituito una risposta illeggibile.'],
  ['Google Calendar rejected the operation', "Google Calendar ha rifiutato l'operazione."],

  // Assistant
  ['Enter a question', 'Inserisci una domanda.'],
  ['Install the search model before indexing mail', 'Installa il modello di ricerca prima di indicizzare la posta.'],
  ['No answer.', 'Nessuna risposta.'],
  ['(Built-in rules: add an AI key in Settings for real answers.) These items match your question:', '(Regole integrate: aggiungi una chiave AI nelle Impostazioni per ottenere vere risposte.) Questi elementi corrispondono alla tua domanda:'],
  ['No action points found.', 'Nessuna attività trovata.'],
  ['The model declined this request.', 'Il modello ha rifiutato questa richiesta.'],
  ['request failed', 'richiesta non riuscita'],

  // Attachment previews
  ['File is larger than 25 MB.', 'Il file supera i 25 MB.'],
  ['Image file (no text to read).', 'File immagine (nessun testo da leggere).'],
  ['Audio file: open it to listen.', 'File audio: aprilo per ascoltarlo.'],
  ['Binary file, no text preview.', 'File binario, nessuna anteprima del testo.'],
  ['Attachment not downloaded.', 'Allegato non scaricato.'],
  ['Attachment has no readable content.', "L'allegato non ha contenuto leggibile."],
  ['No visual preview is available for this file type.', 'Nessuna anteprima visiva disponibile per questo tipo di file.'],
  ['The document is protected and cannot be previewed automatically.', 'Il documento è protetto e non può essere visualizzato automaticamente.'],
  ['The document is protected; some content may be unavailable.', 'Il documento è protetto; alcuni contenuti potrebbero non essere disponibili.'],
  ['The visual preview is no longer available. Open the attachment again.', "L'anteprima visiva non è più disponibile. Apri di nuovo l'allegato."],
  // Saving an attachment (src-tauri/src/zone.rs): the antivirus or the attachment policy deleted it.
  ['Windows security removed the saved file. Check Windows Security.', 'La sicurezza di Windows ha rimosso il file salvato. Controlla Sicurezza di Windows.'],
  ['Invalid preview handle.', 'Identificatore di anteprima non valido.'],
  ['Preview state lock failed', "Impossibile bloccare lo stato dell'anteprima."],
  ['Preview handle unavailable', "Identificatore dell'anteprima non disponibile."],
  ['The document preview worker stopped or exceeded its time budget.', "Il processo di anteprima si è fermato o ha superato il tempo previsto."],
  ['Cannot apply the document preview memory limit', "Impossibile applicare il limite di memoria dell'anteprima documenti."],
  ['PdfCraft CLI could not read any page.', 'PdfCraft CLI non è riuscito a leggere alcuna pagina.'],
  ['PdfCraft CLI did not produce a PNG page.', 'PdfCraft CLI non ha prodotto una pagina PNG.'],

  ['Install the ArtCraft preview engines to open a visual attachment preview.', "Installa i motori di anteprima ArtCraft per aprire un'anteprima visiva dell'allegato."],
  ['The browser preview cannot render document pages. Open the desktop app.', "L'anteprima nel browser non può renderizzare le pagine del documento. Apri l'app desktop."],

  // Gmail notifications
  ['This mail server does not support IDLE; periodic sync remains active', 'Questo server di posta non supporta IDLE; la sincronizzazione periodica resta attiva.'],
  ['Gmail notification response is too large', 'La risposta delle notifiche Gmail è troppo grande.'],
  ['Gmail notification connection closed', 'Connessione delle notifiche Gmail chiusa.'],
  ['Gmail did not start IDLE in time', 'Gmail non ha avviato IDLE in tempo.'],
  ['Gmail did not finish IDLE in time', 'Gmail non ha concluso IDLE in tempo.'],
  ['Gmail rejected the IDLE completion', 'Gmail ha rifiutato la conclusione di IDLE.'],
  ['Unexpected Gmail notification response', 'Risposta inattesa delle notifiche Gmail.'],
  ['Gmail notification session expired', 'Sessione delle notifiche Gmail scaduta.'],

  // IMAP and original messages
  ['Invalid message identifier', 'Identificatore del messaggio non valido.'],
  ['Gmail All Mail is unavailable over IMAP. Enable it in Gmail before printing the complete conversation.', 'La cartella Tutti i messaggi di Gmail non è disponibile via IMAP. Attivala in Gmail prima di stampare la conversazione completa.'],
  ['More than one server message matches these headers. Sync the account to verify its exact location before retrying.', "Più di un messaggio sul server corrisponde a queste intestazioni. Sincronizza l'account per verificarne la posizione esatta prima di riprovare."],
  ['The server did not return the complete message', 'Il server non ha restituito il messaggio completo.'],
  ['The server did not provide a conversation ID', 'Il server non ha fornito un ID di conversazione.'],
  ['Conversation changed during download. Retry printing.', 'La conversazione è cambiata durante il download. Riprova a stampare.'],
  ['A conversation message could not be downloaded', 'Non è stato possibile scaricare un messaggio della conversazione.'],
  ['A conversation message could not be read', 'Non è stato possibile leggere un messaggio della conversazione.'],
  ['The conversation is no longer available on the server', 'La conversazione non è più disponibile sul server.'],
  ['IMAP server has no address', 'Il server IMAP non ha un indirizzo.'],
  ['Enable Gmail All Mail over IMAP to search the complete mailbox', 'Attiva via IMAP la cartella Tutti i messaggi di Gmail per cercare in tutta la casella.'],
  ['Enable "All Mail" in Gmail (Settings, Labels, Show in IMAP) to search the server', 'Attiva «Tutti i messaggi» in Gmail (Impostazioni, Etichette, Mostra in IMAP) per cercare sul server.'],
  ['Enable "All Mail" in Gmail (Settings, Labels, Show in IMAP) to bring older mail into the list', 'Attiva «Tutti i messaggi» in Gmail (Impostazioni, Etichette, Mostra in IMAP) per portare in elenco la posta precedente.'],
  ['Gmail omitted UIDVALIDITY; the history cannot safely resume', 'Gmail non ha indicato UIDVALIDITY: il recupero della posta precedente non può riprendere in modo sicuro.'],
  ['Gmail omitted UIDNEXT; the history cannot safely paginate', 'Gmail non ha indicato UIDNEXT: il recupero della posta precedente non può procedere in modo sicuro.'],
  ['Only Gmail accounts have a server history', 'Solo gli account Gmail hanno uno storico sul server.'],
  ['Only Spam and Trash are fetched on request', 'Su richiesta si recuperano solo Spam e Cestino.'],
  ['Invalid date for marking old mail read', 'Data non valida per segnare come letta la posta vecchia.'],
  // Link to a Qamille server (access wave 2)
  ['This computer is not connected to a Qamille server', 'Questo computer non è collegato a un server Qamille.'],
  ['This edition connects only to its own Qamille server', 'Questa edizione si collega solo al proprio server Qamille.'],
  ['Agree to send content to this server before signing in', 'Prima di accedere conferma di voler inviare i contenuti a questo server.'],
  ['This computer is already connected to this server', 'Questo computer è già collegato a questo server.'],
  ['This edition cannot change server', 'Questa edizione non può cambiare server.'],
  ['Write the server address as https://name.example, without a path', "Scrivi l'indirizzo del server come https://nome.example, senza percorso."],
  ['The server address must start with https://', "L'indirizzo del server deve iniziare con https://."],
  ['The Qamille server does not answer', 'Il server Qamille non risponde.'],
  ['This password is among the most used ones. Choose another one', "Questa password è tra le più usate: scegline un'altra."],
  [
    "The server's certificate is not trusted",
    'Il certificato del server non è considerato attendibile. Se è un server della tua organizzazione, aggiungi il suo certificato di autorità.',
  ],
  ['This address is not a Qamille server', 'Questo indirizzo non è un server Qamille.'],
  ['Wrong username, email or password', 'Username, email o password non corretti.'],
  ['Too many attempts. Try again later', 'Troppi tentativi. Riprova più tardi.'],
  ['The server is busy. Try again in a moment', 'Il server è occupato. Riprova tra poco.'],
  ['The server accepts passwords only over HTTPS', 'Il server accetta le password solo in HTTPS.'],
  ['This device is no longer connected to the server', 'Questo dispositivo non è più collegato al server.'],
  ['Change the temporary password to continue', 'Cambia la password provvisoria per continuare.'],
  ['The account already has its maximum of devices', "L'account ha già il massimo di dispositivi."],
  ['The request is too large for the server', 'La richiesta è troppo grande per il server.'],
  ['The server refused the request', 'Il server ha rifiutato la richiesta.'],
  ['The server does not know this device or account', 'Il server non conosce questo dispositivo o account.'],
  ['The Qamille server answered with an error', 'Il server Qamille ha risposto con un errore.'],
  // Mailbox synchronization (wave 7).
  ['Mailbox synchronization needs an active Qamille subscription', "La sincronizzazione delle caselle riprende con l'abbonamento attivo."],
  ['The server has reached its maximum of synchronized mailboxes', 'Il server ha raggiunto il numero massimo di caselle sincronizzate.'],
  ['The mailbox is too large for the server', 'La casella è troppo grande per il server.'],
  ['The server accepts mailboxes only over HTTPS', 'Il server accetta le caselle solo in HTTPS.'],
  ['The server refused the mailbox item', 'Il server ha rifiutato la casella.'],
  ['This mailbox cannot be shared: its settings or credential are not valid', 'Questa casella non si può condividere: le impostazioni o la credenziale non sono valide.'],
  ['This mailbox is too large for the server', 'La casella è troppo grande per il server.'],
  ['The server does not take more synchronized mailboxes', 'Il server non accetta altre caselle sincronizzate.'],
  ['The server did not accept this mailbox', 'Il server non ha accettato questa casella.'],
  ['This server does not synchronize mailboxes', 'Questo server non sincronizza le caselle.'],
  ['The mailbox changed on another computer; it will be retried', 'La casella è cambiata su un altro computer: riprovo.'],
  ['The mailbox list changed while it was being synchronized; it will be retried', "L'elenco delle caselle è cambiato durante l'allineamento: riprovo."],
  ['Google did not revoke the access', "Google non ha revocato l'accesso."],
  ['Agree to keep the mailboxes on this server first', 'Acconsenti prima a tenere le caselle su questo server.'],
  ['The certificate authority file holds no PEM certificate', "Il file dell'autorità di certificazione non contiene un certificato PEM."],
  ['The server session could not be saved', 'Non è stato possibile salvare la sessione del server.'],
  ['Unknown device', 'Dispositivo sconosciuto.'],
  ['The password must be at least 10 characters long', 'La password deve avere almeno 10 caratteri.'],
  ['This server does not offer sign-up', 'Questo server non permette di creare un account.'],
  ['This server does not offer password recovery', 'Questo server non permette di recuperare la password.'],
  ['Accounts are created by the server administrator', "Gli account li crea l'amministratore del server."],
  ['The code is wrong or expired. Ask for a new one', 'Il codice è sbagliato o scaduto. Chiedine uno nuovo.'],
  ['The password must be 10 to 256 characters long', 'La password deve avere da 10 a 256 caratteri.'],
  ['This username is taken. Choose another one', 'Questo username è già in uso. Scegline un altro.'],
  // Selling Professional: browser sign-in and subscriptions
  ['This server does not offer browser sign-in', "Questo server non permette l'accesso dal browser."],
  ['This server does not describe the account', "Questo server non descrive l'account."],
  ['This server does not sell subscriptions', 'Questo server non vende abbonamenti.'],
  ['Unknown sign-in provider', 'Modo di accesso sconosciuto.'],
  ['The sign-in expired. Sign in again', "L'accesso è scaduto. Accedi di nuovo."],
  ['Unknown payment session', 'Sessione di pagamento sconosciuta.'],
  ['The server sent a page address this app does not open', "Il server ha mandato l'indirizzo di una pagina che l'app non apre."],
  ['An active Qamille subscription is required for compute on the server', 'Per il calcolo sul server serve un abbonamento Qamille attivo.'],
  ['The assistant is temporarily unavailable', "L'assistente è temporaneamente non disponibile."],
  ['The assistant is temporarily unavailable.', "L'assistente è temporaneamente non disponibile."],
  ['The assistant model needs more free memory than this server has right now', "Il modello dell'assistente ha bisogno di più memoria libera di quella che il server ha ora."],
  ['The assistant model needs more free memory than this server has right now.', "Il modello dell'assistente ha bisogno di più memoria libera di quella che il server ha ora."],
  ['The assistant model is being prepared on the server; try again in a few minutes', "Il modello dell'assistente è in preparazione sul server: riprova tra qualche minuto."],
  ['The assistant model is being prepared on the server; try again in a few minutes.', "Il modello dell'assistente è in preparazione sul server: riprova tra qualche minuto."],
  ['The assistant provider has changed: the consent must be given again', "Il server ora usa un altro provider per l'assistente: conferma per continuare."],
  ['The assistant provider has changed: the consent must be given again.', "Il server ora usa un altro provider per l'assistente: conferma per continuare."],
  ['The conversation is too long for the assistant', 'La conversazione è troppo lunga per l’assistente.'],
  ['The conversation is too long for the assistant.', 'La conversazione è troppo lunga per l’assistente.'],
  ['The conversation is too long for the assistant (context length exceeded)', 'La conversazione è troppo lunga per l’assistente.'],
  ['The conversation is too long for the assistant (context length exceeded).', 'La conversazione è troppo lunga per l’assistente.'],
  ['The server declared an assistant provider this app cannot use', "Il server ha dichiarato un provider dell'assistente che l'app non può usare."],
  ['The assistant did not answer', "L'assistente non ha risposto."],
  ['The assistant did not answer.', "L'assistente non ha risposto."],
  ['This device is no longer connected to the server', 'Questo dispositivo non è più collegato al server.'],
  ['This device is no longer connected to the server.', 'Questo dispositivo non è più collegato al server.'],
  ['The compute server is busy; the request will be retried shortly', 'Il server è occupato: riproverò a breve.'],
  ['The compute server is busy; the request will be retried shortly.', 'Il server è occupato: riproverò a breve.'],
  ['This account already has an active subscription', "L'account ha già un abbonamento attivo."],
  ['This account has no subscription to manage', "L'account non ha un abbonamento da gestire."],
  ['The browser came back from another sign-in. Try again', 'Il browser è tornato da un altro accesso. Riprova.'],
  ['Sign-in was cancelled in the browser', "L'accesso è stato annullato nel browser."],
  ['No Qamille account uses this Google address. Ask the administrator, or create an account', "Nessun account Qamille usa questo indirizzo Google. Chiedi all'amministratore, oppure crea un account."],
  ['This account cannot sign in here. Ask the administrator', "Questo account non può accedere qui. Chiedi all'amministratore."],
  ['Google did not confirm this address', 'Google non ha confermato questo indirizzo.'],
  ['An account already uses this address. Sign in with its password', 'Un account usa già questo indirizzo. Accedi con la sua password.'],
  ['Google did not complete the sign-in. Try again', "Google non ha completato l'accesso. Riprova."],
  ['Sign-in stopped', 'Accesso interrotto.'],
  ['No answer from the browser after 5 minutes. Try again', 'Nessuna risposta dal browser dopo 5 minuti. Riprova.'],
  ['The local sign-in address could not be opened', "Non è stato possibile aprire l'indirizzo locale per l'accesso."],
  ['The local sign-in address stopped', "L'indirizzo locale per l'accesso si è fermato."],
  ['The browser could not be opened', 'Non è stato possibile aprire il browser.'],
  ['Your Qamille subscription is not active: compute on the server is paused. Subscribe in Settings → Compute, or continue without a server.', 'Il tuo abbonamento Qamille non è attivo: il calcolo sul server è in pausa. Abbonati da Impostazioni → Calcolo, oppure continua senza server.'],
  ['The search tokens of your plan are used up for this month: indexing and semantic search on the server are paused until they renew.', "I token di ricerca del tuo piano sono finiti per questo mese: indicizzazione e ricerca per significato sul server sono ferme finché non si rinnovano."],
  ['The transcription minutes of your plan are used up for this month: transcription on the server is paused until they renew.', 'I minuti di trascrizione del tuo piano sono finiti per questo mese: la trascrizione sul server è ferma finché non si rinnovano.'],
  ['The assistant tokens of your plan are used up for this month: the assistant on the server is paused until they renew.', "I token dell'assistente del tuo piano sono finiti per questo mese: l'assistente sul server è fermo finché non si rinnovano."],
  ['Mailbox synchronization is paused: the monthly limit of your plan is used up', 'La sincronizzazione delle caselle è ferma: il limite mensile del tuo piano è finito.'],
  ['The Terms and the privacy notice must be accepted to create the account', "Per creare l'account vanno accettati i Termini e l'informativa."],
  ['Mailbox changed during search; retry to complete the results', 'La casella è cambiata durante la ricerca; riprova per completare i risultati.'],
  ['A message could not be downloaded', 'Non è stato possibile scaricare un messaggio.'],
  ['A message could not be parsed', 'Non è stato possibile interpretare un messaggio.'],
  ['A message could not be read', 'Non è stato possibile leggere un messaggio.'],

  // Offline copy
  ['Offline copy could not verify the mailbox identity. Sync again before relying on complete offline coverage.', "La copia offline non ha potuto verificare l'identità della casella. Sincronizza di nuovo prima di contare su una copertura offline completa."],
  ['The mailbox changed during sync. Sync again to finish the offline copy.', 'La casella è cambiata durante la sincronizzazione. Sincronizza di nuovo per completare la copia offline.'],
  ['Gmail did not provide readable headers for some messages. Sync again to finish the offline copy.', 'Gmail non ha fornito intestazioni leggibili per alcuni messaggi. Sincronizza di nuovo per completare la copia offline.'],
  ['Gmail did not provide a message UID', 'Gmail non ha fornito un UID del messaggio.'],
  ['Gmail did not provide message headers', 'Gmail non ha fornito le intestazioni del messaggio.'],
  ['Gmail headers could not be read', 'Impossibile leggere le intestazioni di Gmail.'],
  ["Offline copy is partial: the account's local storage limit has been reached.", "La copia offline è parziale: è stato raggiunto il limite dell'archivio locale dell'account."],
  ['Gmail did not return a complete message', 'Gmail non ha restituito un messaggio completo.'],
  ['Gmail message could not be read', 'Impossibile leggere il messaggio di Gmail.'],
  ['Offline copy is partial: a message exceeds the 64 MiB download safety limit. Its content remains available online.', 'La copia offline è parziale: un messaggio supera il limite di sicurezza di 64 MiB per il download. Il contenuto resta disponibile online.'],
  ["Offline copy is partial: increase this account's storage limit to continue.", "La copia offline è parziale: aumenta il limite di archiviazione di questo account per continuare."],
  ['Offline copy is still being prepared. The next sync continues downloading missing content.', 'La copia offline è ancora in preparazione. La prossima sincronizzazione continuerà a scaricare i contenuti mancanti.'],
  ['Gmail did not return this message', 'Gmail non ha restituito questo messaggio.'],
  ['Invalid offline date', 'Data offline non valida.'],
  ['Gmail did not provide the message body', 'Gmail non ha fornito il corpo del messaggio.'],

  // Mail indexing
  ['Full mailbox indexing queued. Progress is saved automatically; new mail is indexed in the background.', "Indicizzazione completa della casella in coda. L'avanzamento viene salvato automaticamente; la nuova posta viene indicizzata in background."],
  ['Indexing paused', 'Indicizzazione in pausa.'],
  ['Account changed or paused', 'Account modificato o in pausa.'],
  ['Local metadata quota reached; indexing paused', 'Quota locale dei metadati raggiunta; indicizzazione in pausa.'],
  ['Message metadata no longer fits the account quota', "I metadati del messaggio non rientrano più nella quota dell'account."],
  ['Indexing did not commit this message', "L'indicizzazione non ha confermato questo messaggio."],
  ['Message was evicted before its index checkpoint', "Il messaggio è stato rimosso prima del punto di controllo dell'indice."],
  ['No accessible IMAP folders; full indexing is unavailable', "Nessuna cartella IMAP accessibile; l'indicizzazione completa non è disponibile."],
  ['Server omitted UIDVALIDITY; indexing cannot safely resume', "Il server non ha indicato UIDVALIDITY; l'indicizzazione non può riprendere in sicurezza."],
  ['Server omitted UIDNEXT; indexing cannot safely paginate', "Il server non ha indicato UIDNEXT; l'indicizzazione non può proseguire in sicurezza."],
  ['Message changed during indexing; retrying this position', "Il messaggio è cambiato durante l'indicizzazione; nuovo tentativo da questa posizione."],
  ['Message headers unavailable', 'Intestazioni del messaggio non disponibili.'],
  ['Message headers could not be parsed', 'Impossibile interpretare le intestazioni del messaggio.'],
  ['Mailbox identity changed; sync before indexing this message', "L'identità della casella è cambiata; sincronizza prima di indicizzare questo messaggio."],
  ['Message has no verified remote UID', 'Il messaggio non ha un UID remoto verificato.'],
  ['Message size unavailable', 'Dimensione del messaggio non disponibile.'],
  ['Message exceeds the 32 MiB temporary download budget; contents not indexed', 'Il messaggio supera il limite di 32 MiB per il download temporaneo; contenuti non indicizzati.'],
  ['Message body unavailable', 'Corpo del messaggio non disponibile.'],
  ['Message exceeded its declared download size', 'Il messaggio ha superato la dimensione di download dichiarata.'],
  ['Unreadable MIME message', 'Messaggio MIME illeggibile.'],
  ['Unreadable message', 'Messaggio illeggibile.'],
  ['Cannot link an indexed passage to its source', 'Impossibile collegare un passaggio indicizzato alla sua fonte.'],
  ['Message exceeds the 8 MiB vector working budget; source preserved', 'Il messaggio supera il limite di lavoro di 8 MiB per i vettori; la fonte è conservata.'],
  ['No indexable content was found', 'Nessun contenuto indicizzabile trovato.'],
  ['Message was removed while indexing', "Il messaggio è stato rimosso durante l'indicizzazione."],
  ['Message changed while indexing', "Il messaggio è cambiato durante l'indicizzazione."],
  ["Indexing paused: increase this account's storage limit to index more mail", 'Indicizzazione in pausa: aumenta il limite di archiviazione di questo account per indicizzare altra posta.'],
  ['Message has more than 1000 MIME parts; remaining parts were skipped', 'Il messaggio ha più di 1000 parti MIME; le parti restanti sono state saltate.'],
  ['Document worker unavailable', 'Processo dei documenti non disponibile.'],
  ['Document contains too many archive entries', "Il documento contiene troppe voci d'archivio."],
  ['Expanded document exceeds the 4 MiB working budget', 'Il documento espanso supera il limite di lavoro di 4 MiB.'],
  ['Extracted text exceeds the 4 MiB working budget', 'Il testo estratto supera il limite di lavoro di 4 MiB.'],
  ['Message index exceeds its memory budget', "L'indice del messaggio supera il limite di memoria."],

  // Semantic search model
  ['Download cancelled.', 'Download annullato.'],
  ["The semantic search model isn't installed yet. Install it in Settings.", 'Il modello di ricerca semantica non è ancora installato. Installalo nelle Impostazioni.'],
  ['Image encoder returned an unexpected shape', "Il codificatore di immagini ha restituito un formato inatteso."],
  ['Image start token missing', 'Token iniziale immagine mancante.'],
  ['Image end token missing', 'Token finale immagine mancante.'],
  ['Image token missing', 'Token immagine mancante.'],
  ['Image tokenizer placeholders do not match the encoder', 'I segnaposto del tokenizer di immagini non corrispondono al codificatore.'],
  ['Image embedding returned an unexpected dimension', "L'indicizzazione dell'immagine ha restituito una dimensione inattesa."],
  ['The semantic index cannot be read. Its original file is unchanged.', "Impossibile leggere l'indice semantico. Il file originale non è stato modificato."],
  ['The model stopped working.', 'Il modello ha smesso di funzionare.'],
  ['The passage could not be linked to its source', 'Impossibile collegare il passaggio alla sua fonte.'],
  ['Semantic index is empty, so only exact words were matched.', "L'indice semantico è vuoto, quindi sono state cercate solo le parole esatte."],
  ['Empty image', 'Immagine vuota.'],
  ['Embedding frame exceeds its memory budget', "Il blocco di indicizzazione semantica supera il limite di memoria."],
  ['Embedding response exceeds its memory budget', "La risposta dell'indicizzazione semantica supera il limite di memoria."],
  ['Attachment exceeds the embedding memory budget (25 MiB)', "L'allegato supera il limite di memoria dell'indicizzazione semantica (25 MiB)."],
  ['Embedding paused after a worker failure; retry in one minute', 'Indicizzazione semantica in pausa dopo un errore del processo; riprova tra un minuto.'],
  ['Embedding paused: the computer needs more available memory', 'Indicizzazione semantica in pausa: il computer ha bisogno di più memoria disponibile.'],
  ['Embedding worker stopped or exceeded its time budget', "Il processo di indicizzazione semantica si è fermato o ha superato il tempo previsto."],
  ['Embedding worker unavailable', 'Processo di indicizzazione semantica non disponibile.'],
  ['The document reader stopped on this file; the file was skipped', "Il lettore di documenti si è fermato su questo file; il file è stato saltato."],
  ['Indexing hit an unexpected error and will continue.', "L'indicizzazione ha incontrato un errore imprevisto e proseguirà."],
  ['No indexable content was found', 'Nessun contenuto indicizzabile trovato.'],
  ['Embedding input unavailable', "Dati in ingresso dell'indicizzazione semantica non disponibili."],
  ['Embedding output unavailable', "Risultato dell'indicizzazione semantica non disponibile."],
  ['Unknown embedding operation', 'Operazione di indicizzazione semantica sconosciuta.'],

  // Compute server (qamille-server, local or remote)
  ['The compute server is not responding; the request will be retried.', 'Il server di calcolo non risponde; la richiesta verrà ripetuta.'],
  ['The compute server is busy; the request will be retried shortly.', 'Il server di calcolo è occupato; la richiesta verrà ripetuta tra poco.'],
  ['The compute server refused the credentials of this request.', 'Il server di calcolo ha rifiutato le credenziali di questa richiesta.'],
  ['Part of the search index could not be read. Reindex to restore all semantic results.', "Una parte dell'indice di ricerca non è leggibile. Reindicizza per ripristinare tutti i risultati semantici."],
  ['Search by meaning is not available: showing word matches only', 'Ricerca per significato non disponibile: risultati solo per parola.'],
  ['The server uses a different model from the index: reindex to search by meaning again', "Il server usa un modello diverso dall'indice: reindicizza per tornare a cercare per significato."],
  ['Not connected to the Qamille server', 'Non collegato al server Qamille.'],
  ['The remote server does not have the EmbeddingGemma 2 model', 'Il server remoto non ha il modello EmbeddingGemma 2.'],
  ['Models on this server are installed by its administrator.', "I modelli di questo server li installa il suo amministratore."],
  ['The semantic search model is still downloading.', 'Il modello di ricerca semantica è ancora in download.'],
  ['The semantic search model could not be installed.', 'Impossibile installare il modello di ricerca semantica.'],
  ['The compute server does not offer the semantic search model.', 'Il server di calcolo non offre il modello di ricerca semantica.'],
  ['The compute server returned no vector.', 'Il server di calcolo non ha restituito alcun vettore.'],
  ['The compute server returned no vectors.', 'Il server di calcolo non ha restituito alcun vettore.'],
  ['The compute server returned an invalid vector.', 'Il server di calcolo ha restituito un vettore non valido.'],
  ['The compute server returned a different number of vectors.', 'Il server di calcolo ha restituito un numero di vettori diverso da quello richiesto.'],
  ['The compute server returned no passages.', 'Il server di calcolo non ha restituito alcun passaggio.'],
  ['The compute server returned no text.', 'Il server di calcolo non ha restituito alcun testo.'],
  ['The compute server returned an unreadable status.', 'Il server di calcolo ha restituito uno stato illeggibile.'],
  ['The compute server is stopping.', 'Il server di calcolo si sta fermando.'],
  ['The compute task stopped unexpectedly.', "L'operazione del server di calcolo si è fermata in modo imprevisto."],
  ['The request took too long.', 'La richiesta ha impiegato troppo tempo.'],
  ['Image patches have an unexpected shape', "Le porzioni dell'immagine hanno un formato inatteso."],
  ['The image is empty.', "L'immagine è vuota."],
  ['Invalid JSON body for this route.', 'Corpo JSON non valido per questa richiesta.'],
  ['Invalid request.', 'Richiesta non valida.'],
  ['Method not allowed for this route.', 'Metodo non consentito per questa richiesta.'],
  ['Not found.', 'Non trovato.'],
  ['Send a multipart body with `name` and `file`.', 'Invia un corpo multipart con `name` e `file`.'],
  ['The compute server stopped twice on this request; it will be tried again later.', 'Il server di calcolo si è fermato due volte su questa richiesta; verrà riprovata più tardi.'],

  // Transcription in the compute server (whisper.cpp)
  ['The speech recognizer whisper-cli is not installed on the compute server.', 'Sul server di calcolo non è installato il riconoscitore vocale whisper-cli.'],
  ['Install the transcription model before transcribing.', 'Installa il modello di trascrizione prima di trascrivere.'],
  ['Install or reinstall the transcription model: it includes the Whisper engine.', 'Installa o reinstalla il modello di trascrizione: comprende il motore Whisper.'],
  ['The transcription model has the wrong size. Reinstall the model.', 'Il modello di trascrizione ha una dimensione errata. Reinstalla il modello.'],
  ['The transcription model failed its check. Reinstall the model.', 'Il modello di trascrizione non ha superato la verifica. Reinstalla il modello.'],
  ['The installation was cancelled.', "L'installazione è stata annullata."],
  ['Not enough disk space for the transcription model. Free some space and try again.', 'Spazio su disco insufficiente per il modello di trascrizione. Libera spazio e riprova.'],
  ['An installation is already in progress.', "Un'installazione è già in corso."],
  ['Download failed. Check the connection and try again.', 'Download non riuscito. Controlla la connessione e riprova.'],
  ['Download interrupted. Try the installation again.', "Download interrotto. Riprova l'installazione."],
  ['The download is larger than expected.', 'Il download è più grande del previsto.'],
  ['The Whisper engine archive is not valid.', "L'archivio del motore Whisper non è valido."],
  ['The Whisper engine archive cannot be read.', "L'archivio del motore Whisper non è leggibile."],
  ['The Whisper engine archive has an invalid path.', "L'archivio del motore Whisper contiene un percorso non valido."],
  ['The Whisper engine archive has an invalid file name.', "L'archivio del motore Whisper contiene un nome di file non valido."],
  ['The Whisper engine archive exceeds the expected size.', "L'archivio del motore Whisper supera la dimensione prevista."],
  ['The download does not contain whisper-cli.exe.', 'Il download non contiene whisper-cli.exe.'],
  ['Cannot prepare the speech recognizer.', 'Impossibile preparare il riconoscitore vocale.'],
  ['Cannot start the speech recognizer. Reinstall the model.', 'Impossibile avviare il riconoscitore vocale. Reinstalla il modello.'],
  ['The speech recognizer did not complete this audio.', 'Il riconoscitore vocale non ha completato questo blocco audio.'],
  ['Cannot read the state of the speech recognizer.', 'Impossibile leggere lo stato del riconoscitore vocale.'],
  ['The audio took longer than 10 minutes to transcribe. Try a smaller model.', 'La trascrizione del blocco ha superato i 10 minuti. Prova un modello più piccolo.'],
  ['The speech recognizer returned no result.', 'Il riconoscitore vocale non ha restituito alcun risultato.'],
  ['The speech recognizer returned a result that is too large.', 'Il riconoscitore vocale ha restituito un risultato troppo grande.'],
  ['The speech recognizer returned an invalid result.', 'Il riconoscitore vocale ha restituito un risultato non valido.'],
  ['The speech recognizer returned no transcription.', 'Il riconoscitore vocale non ha restituito una trascrizione.'],
  ['The speech recognizer returned invalid timestamps.', 'Il riconoscitore vocale ha restituito tempi non validi.'],
  ['The transcription was cancelled.', 'La trascrizione è stata annullata.'],
  ['This server does not transcribe audio.', "Questo server non trascrive l'audio."],
  ['The audio is not a WAV file.', "L'audio non è un file WAV."],
  ['Send a multipart body with `file` and `model`.', 'Invia un corpo multipart con `file` e `model`.'],
  ['Invalid transcription language.', 'Lingua di trascrizione non valida.'],
  ['Unsupported response_format: use json, verbose_json or text.', 'response_format non supportato: usa json, verbose_json o text.'],
  ['The compute server returned no transcription.', 'Il server di calcolo non ha restituito una trascrizione.'],
  ['Cannot start the transcription request.', 'Impossibile avviare la richiesta di trascrizione.'],
  ['The audio window exceeds the transcription limit.', 'Il blocco audio supera il limite della trascrizione.'],

  // Meeting audio capture and recovery
  ['Audio state is unavailable', "Stato dell'audio non disponibile."],
  ['Audio worker stopped', 'Il processo audio si è fermato.'],
  ['Audio worker stopped unexpectedly. Confirmed audio blocks can be recovered; the final unsaved fraction may be missing.', "Il processo audio si è fermato inaspettatamente. I blocchi audio confermati si possono recuperare; l'ultima parte non salvata potrebbe mancare."],
  ['System audio capture is currently available on Windows', "L'acquisizione dell'audio di sistema è attualmente disponibile su Windows."],
  ['The selected audio source is unavailable', 'La sorgente audio selezionata non è disponibile.'],
  ['The selected audio source was disconnected; choose a device again', 'La sorgente audio selezionata è stata scollegata; scegli di nuovo un dispositivo.'],
  ['Another meeting is being recorded', 'È già in registrazione un altro meeting.'],
  ['No recording is active', 'Nessuna registrazione attiva.'],
  ['Invalid meeting folder', 'Cartella del meeting non valida.'],
  ['This meeting already has audio; recover it or start another meeting', 'Questo meeting ha già un audio; recuperalo o avvia un altro meeting.'],
  ['This audio device uses an unsupported format', 'Questo dispositivo audio usa un formato non supportato.'],
  ['Audio arrived faster than it could be saved. Recording stopped; saved audio can be recovered', "L'audio è arrivato più velocemente di quanto si potesse salvare. Registrazione interrotta; l'audio salvato si può recuperare."],
  ['An audio device disconnected. Recording stopped; saved audio can be recovered', "Un dispositivo audio è stato scollegato. Registrazione interrotta; l'audio salvato si può recuperare."],
  ['Another process owns this recording', 'Questa registrazione appartiene a un altro processo.'],
  ['Disk space is below 256 MiB. Recording stopped; free space before continuing', 'Spazio su disco sotto i 256 MiB. Registrazione interrotta; libera spazio prima di continuare.'],
  ['Less than 1 GiB of disk space remains. Free space before a long recording', 'Resta meno di 1 GiB di spazio su disco. Libera spazio prima di una registrazione lunga.'],
  ['Invalid meeting audio path', 'Percorso audio del meeting non valido.'],
  ['Audio chunk must be a regular file', 'Il blocco audio deve essere un file normale.'],
  ['Audio file is outside this meeting', 'Il file audio è fuori da questo meeting.'],
  ['Audio requests must be 0–120 seconds at 8–96 kHz', 'Le richieste audio devono durare 0–120 secondi a 8–96 kHz.'],
  ['Invalid audio chunk format', 'Formato del blocco audio non valido.'],
  ['An audio block is incomplete', 'Un blocco audio è incompleto.'],
  ['Invalid recovery path', 'Percorso di recupero non valido.'],
  ['Audio integrity receipt is damaged', "La ricevuta di integrità dell'audio è danneggiata."],
  ['Audio checksum or immutable metadata does not match', "Il checksum dell'audio o i metadati immutabili non corrispondono."],
  ['Invalid audio block', 'Blocco audio non valido.'],
  ['An audio checkpoint already exists', 'Esiste già un punto di controllo audio.'],
  ['Audio time overflow', 'Tempo audio fuori intervallo.'],
  ['Cannot identify recording without valid metadata or a numeric meeting folder', 'Impossibile identificare la registrazione senza metadati validi o una cartella meeting numerica.'],
  ['Recording manifest was unavailable; reconstructed from surviving audio files', 'Il manifesto della registrazione non era disponibile; è stato ricostruito dai file audio rimasti.'],
  ['No valid audio frames were found; originals and the loss report were preserved', 'Nessun frame audio valido trovato; gli originali e il rapporto sulle perdite sono stati conservati.'],
  ['Invalid audio sample rate', 'Frequenza di campionamento audio non valida.'],
  ['Audio timestamp overflow', 'Timestamp audio fuori intervallo.'],
  ['Staging WAV header is not trustworthy PCM16', "L'intestazione del WAV temporaneo non è un PCM16 affidabile."],
  ['Staging WAV has no bounded complete PCM frames', 'Il WAV temporaneo non contiene frame PCM completi.'],
  ['Staging WAV changed during recovery', 'Il WAV temporaneo è cambiato durante il recupero.'],
  ['A synchronized audio receipt survived but its complete WAV is unavailable', 'Una ricevuta audio sincronizzata è rimasta, ma il suo WAV completo non è disponibile.'],
  ['Recording was interrupted; the final unsaved fraction may be missing', "La registrazione è stata interrotta; l'ultima parte non salvata potrebbe mancare."],
  ['Recording was interrupted by an audio device or queue error; saved audio was preserved', "La registrazione è stata interrotta da un errore del dispositivo audio o della coda; l'audio salvato è stato conservato."],
  ['Audio receipt identifies a different file', 'La ricevuta audio indica un file diverso.'],
  ['No independent audio timeline metadata is available', 'Non sono disponibili metadati indipendenti sulla sequenza temporale audio.'],
  ['Audio block is still staged', 'Il blocco audio è ancora temporaneo.'],
  ['Audio checkpoint metadata does not match its file', 'I metadati del punto di controllo audio non corrispondono al suo file.'],
  ['Audio checkpoint duration does not match its file', 'La durata del punto di controllo audio non corrisponde al suo file.'],
  ['An audio checkpoint is truncated', 'Un punto di controllo audio è troncato.'],
]);

const plural = (count: string, one: string, many: string) => (count === '1' ? one : many);
const decimal = (value: string) => value.replace('.', ',');
// For a message placed mid-sentence: its translation may end in a period of its own.
const clause = (message: string) => translateLine(message).replace(/\.$/, '');
const indexed = (count: string) =>
  plural(count, '1 messaggio indicizzato', `${count} messaggi indicizzati`);

// Ordered: specific templates first. Captures that hold another backend message go
// through translateLine, so a wrapper never leaves its inner text in English.
const TEMPLATES: [RegExp, Replacement][] = [
  // Index report
  [/^(\d+) messages indexed$/, (count) => indexed(count)],
  [/^Too many attempts\. Try again in (\d+) seconds$/, 'Troppi tentativi. Riprova tra $1 secondi.'],
  [
    /^(\d+) messages indexed; (\d+) skipped because the account storage limit was reached$/,
    (count, skipped) =>
      `${indexed(count)}; ${plural(skipped, '1 saltato', `${skipped} saltati`)} perché è stato raggiunto il limite dell'archivio locale dell'account`,
  ],
  [
    /^(.+)\. (\d+) messages were not indexed because content was unavailable or exceeded the temporary download limit\.$/,
    (result, skipped) =>
      `${translateLine(result)}. ${plural(
        skipped,
        '1 messaggio non è stato indicizzato perché il contenuto non era disponibile o superava il limite di download temporaneo.',
        `${skipped} messaggi non sono stati indicizzati perché il contenuto non era disponibile o superava il limite di download temporaneo.`,
      )}`,
  ],

  // Local storage
  [
    /^Protected mail and drafts need ([\d.]+) GB before the safety reserve\. Choose a larger limit or Unlimited\. No data was removed\.$/,
    (size) =>
      `La posta protetta e le bozze richiedono ${decimal(size)} GB prima della riserva di sicurezza. Scegli un limite più alto o Illimitato. Nessun dato è stato rimosso.`,
  ],
  [
    /^This limit removes approximately ([\d.]+) MB of the oldest replaceable local mail data from this account\. Mail on Gmail and protected drafts stay unchanged\. Apply this limit\?$/,
    (size) =>
      `Questo limite rimuove circa ${decimal(size)} MB dei dati di posta locali più vecchi e sostituibili di questo account. La posta su Gmail e le bozze protette restano invariate. Applicare il limite?`,
  ],
  [
    /^Account (\d+) has no space for this change without removing protected mail or drafts\. Increase its limit or choose Unlimited; this change was not saved\.$/,
    "L'account $1 non ha spazio per questa modifica senza rimuovere posta protetta o bozze. Aumenta il limite o scegli Illimitato; la modifica non è stata salvata.",
  ],

  // Attachment visual previews
  [/^Install (.+) to preview this attachment\.$/, 'Installa $1 per visualizzare questo allegato.'],
  [
    /^The rendered page exceeds the (\d+) MiB preview budget\.$/,
    'La pagina renderizzata supera il limite di anteprima di $1 MiB.',
  ],
  [/^Choose a page between 1 and (\d+)\.$/, 'Scegli una pagina tra 1 e $1.'],
  [/^(.+) CLI failed: ([\s\S]+)$/, '$1 CLI non riuscito: $2'],
  [/^(.+) did not produce a PDF preview\.$/, '$1 non ha prodotto un’anteprima PDF.'],
  [
    /^PdfCraft CLI returned unreadable document information: (.+)$/,
    'PdfCraft CLI ha restituito informazioni sul documento illeggibili: $1',
  ],

  // Italian Rust wrappers can carry an English inner message from the audio layer.
  [
    /^Aggiornamento dell'indice non riuscito: (.+)\. I blocchi audio già scritti restano recuperabili\.$/,
    (error) =>
      `Aggiornamento dell'indice non riuscito: ${clause(error)}. I blocchi audio già scritti restano recuperabili.`,
  ],
  [
    /^Registrazione attiva, ma l'indice non è stato aggiornato: (.+)\. Usa Termina per fermarla\.$/,
    (error) =>
      `Registrazione attiva, ma l'indice non è stato aggiornato: ${clause(error)}. Usa Termina per fermarla.`,
  ],
  [
    /^Verifica dell'audio non completata: (.+)\. I file sono conservati; riprova Recupera audio\.$/,
    (error) =>
      `Verifica dell'audio non completata: ${clause(error)}. I file sono conservati; riprova Recupera audio.`,
  ],
  [/^Verifica audio non riuscita: (.+)$/, (error) => `Verifica audio non riuscita: ${translateLine(error)}`],

  // Wrappers around another message
  [
    /^Changes are saved\. Local cleanup will retry: (.+)$/,
    (error) => `Modifiche salvate. La pulizia locale verrà ritentata: ${translateLine(error)}`,
  ],
  [
    /^(.+) Attachment cleanup also needs retry: (.+)$/,
    (message, error) =>
      `${translateLine(message)} Anche la pulizia degli allegati va ritentata: ${translateLine(error)}`,
  ],
  [
    /^Message was sent but the local copy could not be saved\. Do not send it again\. (.+)$/,
    (error) =>
      `Il messaggio è stato inviato, ma non è stato possibile salvarne la copia locale. Non inviarlo di nuovo. ${translateLine(error)}`,
  ],
  [
    /^Delivery was attempted but its result could not be saved\. Do not resend\. (.+)$/,
    (error) =>
      `L'invio è stato tentato, ma non è stato possibile salvarne il risultato. Non inviarlo di nuovo. ${translateLine(error)}`,
  ],
  [
    /^Delivery result is uncertain; do not resend automatically\. (.+)$/,
    (error) =>
      `L'esito dell'invio è incerto; non inviarlo di nuovo automaticamente. ${translateLine(error)}`,
  ],
  [
    /^Scheduled message ‘(.+)’: (.+)$/,
    (subject, error) => `Messaggio programmato ‘${subject}’: ${translateLine(error)}`,
  ],
  [
    /^Live Gmail sync interrupted; reconnecting automatically\. (.+)$/,
    (error) =>
      `Sincronizzazione Gmail in tempo reale interrotta; riconnessione automatica in corso. ${translateLine(error)}`,
  ],
  [
    /^Sync stopped before completion: (.+)\. Confirmed downloads are retained; sync again to continue\.$/,
    (error) =>
      `Sincronizzazione interrotta prima del completamento: ${clause(error)}. I download confermati sono conservati; sincronizza di nuovo per continuare.`,
  ],
  [
    /^Embedding paused; the last saved index is preserved\. (.+)$/,
    (error) =>
      `Indicizzazione semantica in pausa; l'ultimo indice salvato è conservato. ${translateLine(error)}`,
  ],
  [/^Document parser failed: (.+)$/, 'Lettura del documento non riuscita: $1'],
  [/^Indexing stopped on this message: (.+)$/, "L'indicizzazione si è interrotta su questo messaggio: $1"],
  [/^Message set aside: (.+)$/, (error) => `Messaggio messo da parte: ${translateLine(error)}`],
  [/^Message skipped after (\d+) attempts: (.+)$/, (attempts, error) => `Messaggio saltato dopo ${attempts} tentativi: ${translateLine(error)}`],
  [/^AI error: (.+)$/, (error) => `Errore AI: ${translateLine(error)}`],
  [
    /^(.+) \(Google said: (.+)\)$/,
    (hint, said) => `${translateLine(hint)} (Google ha risposto: ${said})`,
  ],
  // Sync and server search report each failure as "<account email>: <message>".
  [/^([^\s:@]+@[^\s:@]+): (.+)$/, (email, error) => `${email}: ${translateLine(error)}`],

  // Mail and search
  [/^Attachment: (.+)$/, 'Allegato: $1'],
  [/^Attachment unavailable: (.+)$/, 'Allegato non disponibile: $1'],
  [/^The mailbox (.+) was removed or switched off on another computer$/, 'La casella $1 è stata rimossa o esclusa dalla sincronizzazione su un altro computer.'],
  [/^Attachment text unavailable: (.+)$/, "Testo dell'allegato non disponibile: $1"],
  [/^The radius must be in days\. Use \/(\d+)d$/, 'Il raggio va espresso in giorni. Usa /$1d.'],
  [
    /^Original message not found on the server: (.+)\. Sync mail and retry\.$/,
    'Messaggio originale non trovato sul server: $1. Sincronizza la posta e riprova.',
  ],
  [
    /^The (.+) folder is unavailable over IMAP\. Enable it in Gmail to complete this search\.$/,
    (folder) =>
      `La cartella ${FOLDERS[folder] ?? folder} non è disponibile via IMAP. Attivala in Gmail per completare questa ricerca.`,
  ],
  [
    /^Offline copy is partial: Gmail folders (.+) are not exposed over IMAP\. Enable them in Gmail and sync again\.$/,
    'La copia offline è parziale: le cartelle Gmail $1 non sono esposte via IMAP. Attivale in Gmail e sincronizza di nuovo.',
  ],
  [/^IMAP connect: (.+)$/, 'Connessione IMAP: $1'],
  [/^IMAP TLS: (.+)$/, 'TLS IMAP: $1'],
  [/^IMAP login: (.+)$/, 'Accesso IMAP: $1'],
  [/^Gmail notifications: (.+)$/, 'Notifiche Gmail: $1'],
  [
    /^Mail data cannot be read; the original file is unchanged: (.+)$/,
    'Impossibile leggere i dati della posta; il file originale non è stato modificato: $1',
  ],
  [
    /^Mail data cannot be read\. The original file is unchanged: (.+)$/,
    'Impossibile leggere i dati della posta. Il file originale non è stato modificato: $1',
  ],
  [/^Invalid sender: (.+)$/, 'Mittente non valido: $1'],
  [/^Invalid recipient: (.+)$/, 'Destinatario non valido: $1'],

  // Sign-in
  [/^Google said: (.+)$/, 'Google ha risposto: $1'],
  [/^Google refused the sign-in: (.+)$/, "Google ha rifiutato l'accesso: $1"],
  [/^Couldn't reach Google: (.+)$/, 'Impossibile raggiungere Google: $1'],
  [/^Google's reply couldn't be read: (.+)$/, 'Impossibile leggere la risposta di Google: $1'],
  [/^Couldn't open the browser: (.+)$/, 'Impossibile aprire il browser: $1'],

  // Mail actions and dates
  [/^No email with id (.+)$/, 'Nessuna email con id $1'],
  [/^(\S+) needs a value$/, 'Questa azione richiede un valore ($1).'],
  [/^A reminder can be at most (\d+) days ahead\.$/, 'Un promemoria può essere al massimo a $1 giorni da oggi.'],
  [/^Unknown action (.+)\. Known: (.+)$/, 'Azione sconosciuta $1. Azioni note: $2'],
  [/^Unknown time zone: (.+)$/, 'Fuso orario sconosciuto: $1'],
  [/^No valid time near (.+) in (.+)$/, 'Nessun orario valido vicino a $1 in $2'],
  [/^The time (.+) does not exist in (.+)\. Choose another time\.$/, "L'orario $1 non esiste in $2. Scegli un altro orario."],

  // Offline assistant answers: the mail content in the capture stays as written.
  [/^Message \[(\d+)\] (.+?) from (\S+): (.*)$/, 'Messaggio [$1] $2 da $3: $4'],
  [/^Event (.+?) on (.+?): (.*)$/, 'Evento $1 il $2: $3'],

  // Semantic search model
  [
    /^Mail data cannot be read; the semantic index is unchanged: (.+)$/,
    "Impossibile leggere i dati della posta; l'indice semantico non è stato modificato: $1",
  ],
  [
    /^Semantic indexing paused: (.+) needs more local space for an atomic index update\. Existing mail and the last saved index are preserved\.$/,
    "Indicizzazione semantica in pausa: $1 ha bisogno di più spazio locale per aggiornare l'indice in modo atomico. La posta esistente e l'ultimo indice salvato sono conservati.",
  ],
  [/^Download failed: (.+)$/, 'Download non riuscito: $1'],
  [/^Download interrupted: (.+)$/, 'Download interrotto: $1'],
  [/^(.+) exceeded its declared size; the download was stopped\.$/, '$1 ha superato la dimensione dichiarata; il download è stato interrotto.'],
  [/^(.+) failed its integrity check; nothing was installed\.$/, '$1 non ha superato il controllo di integrità; non è stato installato nulla.'],
  [/^Couldn't start the model runtime: (.+)$/, 'Impossibile avviare il runtime del modello: $1'],
  [/^Couldn't load the model: (.+)$/, 'Impossibile caricare il modello: $1'],
  [/^Couldn't load the tokenizer: (.+)$/, 'Impossibile caricare il tokenizer: $1'],
  [/^Embedding failed: (.+)$/, 'Indicizzazione semantica non riuscita: $1'],
  [/^The model returned (\d+) values, expected (\d+)\.$/, 'Il modello ha restituito $1 valori, attesi $2.'],
  [/^Image encoder failed: (.+)$/, 'Codificatore di immagini non riuscito: $1'],
  [/^Image embedding failed: (.+)$/, "Indicizzazione dell'immagine non riuscita: $1"],
  [
    /^Image unavailable or exceeds decoding memory budget: (.+)$/,
    'Immagine non disponibile o oltre il limite di memoria per la decodifica: $1',
  ],
  [/^Cannot apply embedding memory limit: (.+)$/, "Impossibile applicare il limite di memoria dell'indicizzazione semantica: $1"],
  [/^Unsupported embedding modality: (.+)$/, 'Tipo di contenuto non supportato per l\'indicizzazione semantica: $1'],
  [/^The compute server returned (\d+) values, expected (\d+)\.$/, 'Il server di calcolo ha restituito $1 valori, attesi $2.'],
  [/^The compute server answered (\d+)\.$/, 'Il server di calcolo ha risposto $1.'],
  [/^This server serves only (.+)\.$/, 'Questo server offre solo $1.'],
  [/^This server serves (\d+) dimensions\.$/, 'Questo server offre vettori a $1 dimensioni.'],
  [/^Send between 1 and (\d+) inputs\.$/, 'Invia da 1 a $1 testi.'],
  [/^Cannot start the document reader: (.+)$/, 'Impossibile avviare il lettore dei documenti: $1'],
  [/^Cannot start the installation: (.+)$/, "Impossibile avviare l'installazione: $1"],
  [/^This server does not offer the transcription model (.*)\.$/, 'Questo server non offre il modello di trascrizione $1.'],
  [/^Cannot write the transcription model: (.+)$/, 'Impossibile scrivere il modello di trascrizione: $1'],
  [/^Cannot install the Whisper engine: (.+)$/, 'Impossibile installare il motore Whisper: $1'],
  [/^cannot (write|lock|open) the (discovery|lock) file: (.+)$/, (verb, file, detail) =>
    `impossibile ${({ write: 'scrivere', lock: 'bloccare', open: 'aprire' } as Record<string, string>)[verb]} il file ${file === 'lock' ? 'di blocco' : 'di scoperta'}: ${detail}`],
  [/^cannot create the data folder: (.+)$/, 'impossibile creare la cartella dei dati: $1'],
  [/^([^;]+): exceeds the 25 MiB per-attachment memory budget$/, '$1: supera il limite di memoria di 25 MiB per allegato'],
  [/^([^;]+): no extractable text; content coverage is partial$/, '$1: nessun testo estraibile; copertura dei contenuti parziale'],
  [/^([^;]+): combined extracted text exceeds the 8 MiB message budget$/, '$1: il testo estratto complessivo supera il limite di 8 MiB per messaggio'],
  [/^Calendar request did not complete: (.+)$/, 'La richiesta al calendario non è stata completata: $1'],

  // Meeting audio capture and recovery
  [
    /^Recovered (\d+) complete audio frames from interrupted staging file (.+)$/,
    (count, name) =>
      `${plural(count, 'Recuperato 1 frame audio completo', `Recuperati ${count} frame audio completi`)} dal file temporaneo interrotto ${name}`,
  ],
  [
    /^Recovered audio block (.+) not yet present in the manifest$/,
    'Recuperato il blocco audio $1, non ancora presente nel manifesto.',
  ],
  [
    /^Recovered (\d+) audio blocks omitted from the recording index$/,
    (count) => plural(count, 'Recuperato 1 blocco audio non ancora indicizzato.', `Recuperati ${count} blocchi audio non ancora indicizzati.`),
  ],
  [/^Audio device unavailable: (.+)$/, 'Dispositivo audio non disponibile: $1'],
  [/^Could not start audio: (.+)$/, "Impossibile avviare l'audio: $1"],
  [/^Unsupported audio sample format: (.+)$/, 'Formato dei campioni audio non supportato: $1'],
  [/^Could not read or write WAV audio: (.+)$/, "Impossibile leggere o scrivere l'audio WAV: $1"],
  [
    /^Recording manifest (.+) is unreadable or belongs to another meeting; original retained$/,
    'Il manifesto della registrazione $1 è illeggibile o appartiene a un altro meeting; originale conservato.',
  ],
  [
    /^Recording metadata recovered from (.+); the final part may be incomplete$/,
    "Metadati della registrazione recuperati da $1; l'ultima parte potrebbe essere incompleta.",
  ],
  [
    /^Repaired inconsistent manifest metadata for (.+) from its audio receipt or native filename$/,
    'Metadati incoerenti del manifesto riparati per $1 dalla ricevuta audio o dal nome file nativo.',
  ],
  [
    /^Unidentified audio staging file (.+) was retained in quarantine; its timeline cannot be inferred$/,
    'Il file audio temporaneo non identificato $1 è stato conservato in quarantena; non è possibile ricostruirne la sequenza temporale.',
  ],
];

// English for messages Rust writes in Italian, mostly meeting code. Stored titles such
// as "Riunione del {}" are deliberately absent: once saved they are editable user data.
const EXACT_ENGLISH = new Map<string, string>([
  // Text Rust chooses by regional locale: these use the exact English Rust literal.
  ['Risultati locali: parte della posta sul server potrebbe mancare. Cerca sul server per completare questa ricerca.', 'Local results: some mail on the server may be missing. Search the server to complete this search.'],
  ['Risultati parziali: alcuni messaggi o allegati non hanno testo ricercabile disponibile.', 'Partial results: some message or attachment text is unavailable.'],
  ['Server controllato per questa ricerca. I risultati descrivono la posta disponibile al momento del controllo.', 'Server checked for this search. Results reflect mail available at the time of the check.'],
  ['Nel testo', 'In message text'],
  ['Corrisponde ai filtri', 'Matches the filters'],
  ['Fuso non riconosciuto: uso UTC / Unknown time zone: using UTC', 'Unknown time zone: using UTC'],

  // Records looked up by id (AppError::missing with an Italian kind)
  ['Attività not found', 'Action item not found'],
  ['Evento not found', 'Event not found'],
  ['Lavoro not found', 'Job not found'],
  ['Passaggio not found', 'Passage not found'],
  ['Trascrizione not found', 'Transcript not found'],
  ['Verbale not found', 'Summary not found'],
  ['Versione sintesi not found', 'Summary version not found'],
  ['Versione trascrizione not found', 'Transcript version not found'],
  ['Lavoro del risultato recuperato not found', 'Recovered result job not found'],

  // Meeting service
  ['Data non valida.', 'Invalid date.'],
  ['Il meeting è stato modificato. La tua bozza è conservata: rileggi la versione aggiornata.', 'This meeting was changed. Your draft is kept: review the updated version.'],
  ['Indice della registrazione non disponibile', 'Recording index unavailable'],
  ['Identificativo di avvio non valido.', 'Invalid start identifier.'],
  ['Scegli almeno una sorgente audio.', 'Choose at least one audio source.'],
  ['Recorder non disponibile', 'Recorder unavailable'],
  ['Una registrazione è già in corso.', 'A recording is already in progress.'],
  ['La registrazione è stata interrotta. Puoi recuperare i blocchi salvati.', 'The recording was interrupted. You can recover the saved audio chunks.'],
  ['Elaborazione interrotta: riprova per continuare.', 'Processing interrupted: retry to continue.'],
  ['Termina prima la registrazione.', 'Stop the recording first.'],
  ['Appunti troppo lunghi.', 'Notes are too long.'],
  ['Posizione audio non valida.', 'Invalid audio position.'],
  ['Inserisci un titolo.', 'Enter a title.'],
  ['Lavori non disponibili', 'Jobs unavailable'],
  ['Annullamento in corso. Riprova a eliminare quando il lavoro è terminato.', 'Cancelling. Try deleting again when the job has finished.'],
  ['Intervallo audio massimo: 30 secondi.', 'Maximum audio range: 30 seconds.'],
  ['Importa o ritrascrivi il vecchio file per ascoltarlo nel player.', 'Import or re-transcribe the old file to listen to it in the player.'],
  ['Scegli almeno un contenuto da esportare.', 'Choose at least one item to export.'],
  ['Modello non disponibile.', 'Model unavailable.'],
  ['Modelli non disponibili', 'Models unavailable'],
  ['Un download è già in corso.', 'A download is already in progress.'],
  ['Provider non valido.', 'Invalid provider.'],
  ["Termina la registrazione prima di elaborare l'audio.", 'Stop the recording before processing the audio.'],
  ['Aggiungi una trascrizione o degli appunti.', 'Add a transcript or notes.'],
  ["Un'elaborazione è già in corso per questo meeting.", 'Processing is already running for this meeting.'],
  ['File non disponibile', 'File unavailable'],
  ['Audio non disponibile.', 'Audio unavailable.'],
  ["L'elaborazione si è interrotta. Il risultato precedente è conservato.", 'Processing stopped. The previous result is kept.'],
  ["Il contenuto è cambiato durante l'elaborazione. Il risultato precedente è conservato; riprova.", 'The content changed during processing. The previous result is kept; try again.'],
  ['Questo lavoro non richiede un nuovo tentativo.', 'This job does not need a retry.'],
  ['Scegli nuovamente il file con Importa audio.', 'Choose the file again with Import audio.'],
  ['Formato non supportato.', 'Unsupported format.'],

  // Meeting storage and recovery
  ["Audio recuperato fino all'ultimo blocco disponibile. Registrazione incompleta: alcune parti possono mancare.", 'Audio recovered up to the last available chunk. Recording incomplete: some parts may be missing.'],
  ["L'indice del meeting è danneggiato o mancante. I file originali sono conservati; la registrazione è ricostruita dai blocchi disponibili. Verifica titolo, appunti e versioni.", 'The meeting index is damaged or missing. The original files are kept; the recording is rebuilt from the available audio chunks. Check the title, notes and versions.'],
  ['Nessun blocco audio confermato trovato; i file originali restano conservati.', 'No confirmed audio chunks found; the original files are still kept.'],
  ['Formato meeting non supportato.', 'Unsupported meeting format.'],
  ['Questo meeting esiste già.', 'This meeting already exists.'],
  ['Identità o formato meeting non modificabile.', 'Meeting identity or format cannot be changed.'],
  ['Confermare prima la rimozione del meeting.', 'Confirm the meeting removal first.'],
  ["La registrazione è ancora attiva; l'audio non è stato rimosso.", 'The recording is still active; the audio was not removed.'],
  ['Indice meeting danneggiato.', 'Meeting index is damaged.'],
  ['Formato meeting non supportato: il file originale non viene modificato.', 'Unsupported meeting format: the original file is not changed.'],
  ['Indice meeting incompleto.', 'Meeting index is incomplete.'],
  ['Identificativo meeting incoerente.', 'Inconsistent meeting identifier.'],
  ['Verifica dei risultati AI non riuscita: i file sono conservati.', 'AI result check failed: the files are kept.'],
  ["Risultato AI in attesa di salvataggio: ripristina l'accesso al disco e riapri il meeting. L'audio è conservato.", 'AI result waiting to be saved: restore disk access and reopen the meeting. The audio is kept.'],
  ['Risultato AI non disponibile', 'AI result unavailable'],
  ['Installazione interrotta. Puoi riprovare.', 'Installation interrupted. You can retry.'],
  ["Impossibile avviare l'installazione. Puoi riprovare.", 'Could not start the installation. You can retry.'],
  ["Impossibile avviare l'installazione.", 'Could not start the installation.'],
  ["Impossibile avviare l'elaborazione. Puoi riprovare.", 'Could not start processing. You can retry.'],
  ['Risultato AI incompleto', 'Incomplete AI result'],
  ['Checksum del risultato AI non valido', 'Invalid AI result checksum'],
  ['Formato del risultato AI non valido', 'Invalid AI result format'],
  ['Identità del risultato AI non valida', 'Invalid AI result identity'],
  ["Questa cartella è già aperta in un'altra interfaccia Qamille.", 'This folder is already open in another Qamille window.'],

  // Audio import and jobs
  ['Formato di esportazione non supportato. Usa Markdown, TXT o JSON.', 'Unsupported export format. Use Markdown, TXT or JSON.'],
  ['Il file audio è vuoto.', 'The audio file is empty.'],
  ['Il file è cambiato durante la copia. Riprova quando è stato salvato.', 'The file changed while it was being copied. Try again once it has been saved.'],
  ['Formato audio non supportato o file non valido. Usa WAV, MP3, M4A/AAC o FLAC.', 'Unsupported audio format or invalid file. Use WAV, MP3, M4A/AAC or FLAC.'],
  ['Nessuna traccia audio nel file.', 'The file has no audio track.'],
  ['Il codec del file non è supportato.', "The file's codec is not supported."],
  ['Lettura del file audio interrotta: nessun risultato è stato pubblicato.', 'Reading the audio file was interrupted: no result was published.'],
  ['Il file contiene un blocco audio danneggiato.', 'The file contains a damaged audio chunk.'],
  ['Configurazione audio non supportata.', 'Unsupported audio configuration.'],
  ['Il file cambia frequenza di campionamento durante la traccia.', 'The file changes sample rate during the track.'],
  ['Blocco audio oltre il limite di memoria previsto.', 'The audio chunk exceeds the expected memory limit.'],
  ['Il file contiene campioni audio non validi.', 'The file contains invalid audio samples.'],
  ['Il file non contiene campioni audio decodificabili.', 'The file contains no decodable audio samples.'],
  ['Il file termina prima della durata dichiarata. Verifica che sia completo.', 'The file ends before its declared duration. Check that it is complete.'],
  ['Operazione annullata.', 'Operation cancelled.'],
  ['Impossibile creare un identificatore di lavoro.', 'Could not create a job identifier.'],
  ["Spazio su disco insufficiente per completare l'operazione.", 'Not enough disk space to complete the operation.'],
  ["Spazio su disco insufficiente. Libera spazio e riprova; l'audio esistente è conservato.", 'Not enough disk space. Free up space and try again; the existing audio is kept.'],
  ['Campione audio non valido.', 'Invalid audio sample.'],

  // Speech models
  ['Whisper Tiny · multilingue', 'Whisper Tiny · multilingual'],
  ['Whisper Base · multilingue', 'Whisper Base · multilingual'],
  ['Whisper Small · multilingue', 'Whisper Small · multilingual'],
  ['Modello di trascrizione non supportato.', 'Unsupported transcription model.'],
  ['Installa o reinstalla il modello locale: comprende il motore Whisper.', 'Install or reinstall the local model: it includes the Whisper engine.'],
  ['Dimensione del modello non valida. Reinstalla il modello.', 'Invalid model size. Reinstall the model.'],
  ['Verifica del download fallita. Nessun modello non verificato è stato attivato.', 'Download verification failed. No unverified model was activated.'],
  ['Download non riuscito. Controlla la connessione e riprova.', 'Download failed. Check the connection and try again.'],
  ["Download interrotto. Riprova l'installazione.", 'Download interrupted. Try the installation again.'],
  ['Download più grande del previsto.', 'The download is larger than expected.'],
  ['Il motore locale integrato è disponibile per Windows x64.', 'The built-in local engine is available for Windows x64.'],
  ["Un'installazione è già in corso.", 'An installation is already in progress.'],
  ['Archivio del motore non valido.', 'Invalid engine archive.'],
  ['Archivio del motore non leggibile.', 'The engine archive cannot be read.'],
  ["Percorso non valido nell'archivio del motore.", 'Invalid path in the engine archive.'],
  ['Nome file del motore non valido.', 'Invalid engine file name.'],
  ['Archivio motore oltre il limite previsto.', 'The engine archive exceeds the expected limit.'],
  ['Il download non contiene whisper-cli.exe.', 'The download does not contain whisper-cli.exe.'],

  // Transcription
  ['Non ci sono blocchi audio da trascrivere.', 'There are no audio chunks to transcribe.'],
  ['Lingua di trascrizione non valida.', 'Invalid transcription language.'],
  ['La registrazione è vuota.', 'The recording is empty.'],
  ['Installa il modello locale prima della trascrizione.', 'Install the local model before transcribing.'],
  ['La trascrizione remota richiede un endpoint compatibile OpenAI configurato.', 'Remote transcription needs a configured OpenAI-compatible endpoint.'],
  ['Nome del modello remoto non valido.', 'Invalid remote model name.'],
  ['Provider di trascrizione sconosciuto.', 'Unknown transcription provider.'],
  ['Impossibile salvare il blocco di trascrizione.', 'Could not save the transcript chunk.'],
  ['Impossibile avviare il motore locale. Reinstalla il modello.', 'Could not start the local engine. Reinstall the model.'],
  ["Il motore locale non ha completato questo blocco. L'audio rimane salvato.", 'The local engine did not complete this chunk. The audio stays saved.'],
  ['Impossibile leggere lo stato del motore locale.', 'Could not read the status of the local engine.'],
  ['Trascrizione del blocco oltre il limite di 10 minuti. Prova un modello più piccolo.', 'Transcribing this chunk exceeded the 10-minute limit. Try a smaller model.'],
  ['Risposta del motore locale troppo grande.', 'The local engine response is too large.'],
  ['Risposta del motore locale non valida.', 'Invalid local engine response.'],
  ['La risposta locale non contiene una trascrizione valida.', 'The local response does not contain a valid transcript.'],
  ['Timestamp del motore non valido.', 'Invalid engine timestamp.'],
  ['Blocco audio remoto oltre il limite interno.', 'The remote audio chunk exceeds the internal limit.'],
  ['Il servizio di trascrizione non è raggiungibile.', 'The transcription service cannot be reached.'],
  ['Risposta del servizio di trascrizione non valida.', 'Invalid response from the transcription service.'],
  ['Il servizio non ha restituito testo o segmenti.', 'The service returned no text or segments.'],
  // Simple Meetings page: readiness, default transcription, audio cleanup, Settings
  ['Installa il modello di trascrizione in Impostazioni.', 'Install the transcription model in Settings.'],
  ['Configura un endpoint compatibile OpenAI in Impostazioni per trascrivere in remoto.', 'Set up an OpenAI-compatible endpoint in Settings to transcribe remotely.'],
  ['Trascrizione non configurata.', 'Transcription is not set up.'],
  ["L'audio di questa riunione è stato eliminato dopo la trascrizione.", 'The audio of this meeting was deleted after transcription.'],
  ["Non c'è audio da eliminare.", 'There is no audio to delete.'],
  ['Scegli se trascrivere su questo computer o con il servizio remoto.', 'Choose whether to transcribe on this computer or with the remote service.'],
  ['Scegli se trascrivere con il server di calcolo o con un servizio esterno compatibile OpenAI.', 'Choose whether to transcribe with the compute server or an external OpenAI-compatible service.'],
  ['Il server di calcolo non offre questo modello di trascrizione.', 'The compute server does not offer this transcription model.'],

  // Meeting summary. The interface calls the "verbale" a summary, so these do too.
  ['Configura un modello per generare il verbale. Gli appunti sono conservati.', 'Set up a model to generate the summary. Your notes are kept.'],
  ['Lingua del verbale non supportata.', 'Unsupported summary language.'],
  ['Completa o correggi la trascrizione prima di generare un verbale completo.', 'Complete or correct the transcript before generating a full summary.'],
  ['Aggiungi appunti o una trascrizione prima del verbale.', 'Add notes or a transcript before the summary.'],
  ['Sintesi finale non valida. Il verbale precedente è conservato.', 'Invalid final summary. The previous summary is kept.'],
  ['Sintesi finale fuori dai limiti previsti.', 'The final summary is outside the expected limits.'],
  ['Il modello non ha ridotto i blocchi: impossibile completare il verbale senza troncare contenuti.', 'The model did not shorten the chunks: the summary cannot be completed without cutting content.'],
  ['Riunione troppo lunga per completare la sintesi con questo modello.', 'This meeting is too long to complete the summary with this model.'],
  ['Blocco di fonti non valido.', 'Invalid source chunk.'],
  ['Il contesto del modello è troppo piccolo anche per un blocco minimo. Scegli un modello con un contesto più ampio.', "The model's context is too small even for a minimal chunk. Choose a model with a larger context."],
  ['Impossibile preparare le fonti del verbale.', 'Could not prepare the summary sources.'],
  ['Il modello non ha restituito un verbale strutturato valido.', 'The model did not return a valid structured summary.'],
  ['Il verbale non rispetta i limiti di struttura.', 'The summary does not respect the structure limits.'],
  ['Responsabile o scadenza non presenti nelle fonti citate.', 'Owner or due date not found in the cited sources.'],
  ['Decisione o attività senza fonte verificabile.', 'Decision or action item without a verifiable source.'],
  ['Il verbale cita un passaggio inesistente o una versione diversa.', 'The summary cites a passage that does not exist or a different version.'],
  ['Il blocco supera il contesto del modello. Scegli un modello con un contesto più ampio.', "The chunk exceeds the model's context. Choose a model with a larger context."],
  ['Il modello non ha completato il verbale. Riprova; i risultati precedenti sono conservati.', 'The model did not complete the summary. Try again; the previous results are kept.'],
  ['Un blocco del verbale supera il budget previsto.', 'A summary chunk exceeds the expected budget.'],
]);

const clauseEnglish = (message: string) => translateLineEnglish(message).replace(/\.$/, '');

const TEMPLATES_ENGLISH: [RegExp, Replacement][] = [
  [/^Allegato: (.+)$/, 'Attachment: $1'],
  [
    /^Aggiornamento dell'indice non riuscito: (.+)\. I blocchi audio già scritti restano recuperabili\.$/,
    (error) =>
      `Index update failed: ${clauseEnglish(error)}. Audio chunks already written can still be recovered.`,
  ],
  [
    /^Registrazione attiva, ma l'indice non è stato aggiornato: (.+)\. Usa Termina per fermarla\.$/,
    (error) =>
      `Recording is active, but the index was not updated: ${clauseEnglish(error)}. Use Stop and save to end it.`,
  ],
  [
    /^Verifica dell'audio non completata: (.+)\. I file sono conservati; riprova Recupera audio\.$/,
    (error) =>
      `Audio check not completed: ${clauseEnglish(error)}. The files are kept; try Recover audio again.`,
  ],
  [/^Verifica audio non riuscita: (.+)$/, (error) => `Audio check failed: ${translateLineEnglish(error)}`],
  [
    /^Metadati recuperati dalla copia (.+?)\. L'ultima modifica potrebbe essere incompleta; l'audio viene verificato separatamente\.$/,
    'Metadata recovered from copy $1. The last change may be incomplete; the audio is checked separately.',
  ],
  [
    /^Risultato AI (.+) danneggiato: il file è conservato; puoi riprovare l'elaborazione\.$/,
    'AI result $1 is damaged: the file is kept; you can retry the processing.',
  ],
  [
    /^Il servizio di trascrizione ha risposto con errore (\d+)\. L'audio rimane salvato\.$/,
    'The transcription service responded with error $1. The audio stays saved.',
  ],
  // Offline assistant answers: the meeting content in the captures stays as written.
  [
    /^Meeting \[(\d+)\] (.+?) — fonte (.+?) \/ versione (.+?) \/ posizione audio (.+?) ms: (.*)$/,
    (id, title, source, version, position, text) =>
      `Meeting [${id}] ${title} — source ${source} / version ${version} / audio position ${position === 'non disponibile' ? 'unavailable' : position} ms: ${text}`,
  ],
];

function lookup(line: string, exact: Map<string, string>, templates: [RegExp, Replacement][]): string | null {
  const known = exact.get(line);
  if (known !== undefined) return known;
  for (const [pattern, replacement] of templates) {
    const match = pattern.exec(line);
    if (!match) continue;
    return typeof replacement === 'string'
      ? line.replace(pattern, replacement)
      : replacement(...match.slice(1));
  }
  return null;
}

function translateLine(line: string): string {
  if (!line || line.length > MAX_LINE_LENGTH) return line;
  const translated = lookup(line, EXACT, TEMPLATES);
  if (translated !== null) return translated;
  return line.includes('; ') ? translateJoined(line) : line;
}

// Index warnings are stored joined with "; ", and some warnings contain "; " themselves,
// so the longest run of parts that forms a known message wins.
function translateJoined(line: string): string {
  const parts = line.split('; ');
  if (parts.length > MAX_JOINED_PARTS) return line;
  const output: string[] = [];
  for (let start = 0; start < parts.length; ) {
    let end = start + 1;
    let text = parts[start];
    for (let candidate = parts.length; candidate > start; candidate--) {
      const known = lookup(parts.slice(start, candidate).join('; '), EXACT, TEMPLATES);
      if (known === null) continue;
      end = candidate;
      text = end < parts.length ? known.replace(/\.$/, '') : known;
      break;
    }
    output.push(text);
    start = end;
  }
  return output.join('; ');
}

function translateLineEnglish(line: string): string {
  if (!line || line.length > MAX_LINE_LENGTH) return line;
  return lookup(line, EXACT_ENGLISH, TEMPLATES_ENGLISH) ?? line;
}

export function backendTextItalian(message: string): string {
  if (typeof message !== 'string') return message;
  return message.split('\n').map(translateLine).join('\n');
}

export function backendTextEnglish(message: string): string {
  if (typeof message !== 'string') return message;
  return message.split('\n').map(translateLineEnglish).join('\n');
}
