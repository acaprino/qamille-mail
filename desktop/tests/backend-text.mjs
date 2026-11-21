import assert from 'node:assert/strict';
import test from 'node:test';
import { backendTextEnglish, backendTextItalian } from '../src/lib/backend-text.ts';

test('exact messages are translated, including final punctuation variants', () => {
  assert.equal(backendTextItalian('Add at least one recipient'), 'Aggiungi almeno un destinatario.');
  assert.equal(backendTextItalian('Draft not found'), 'Bozza non trovata.');
  assert.equal(backendTextItalian('Choose a positive account storage limit or Unlimited.').startsWith('Scegli'), true);
  assert.equal(backendTextItalian('Add at least one recipient.'), 'Add at least one recipient.');
});

test('templates keep their captures', () => {
  assert.equal(backendTextItalian('Attachment unavailable: report.pdf'), 'Allegato non disponibile: report.pdf');
  assert.equal(backendTextItalian('Attachment: Q4 $1.pdf'), 'Allegato: Q4 $1.pdf');
  assert.equal(
    backendTextItalian('The time 02:30 does not exist in Europe/Rome. Choose another time.'),
    "L'orario 02:30 non esiste in Europe/Rome. Scegli un altro orario.",
  );
  assert.equal(
    backendTextItalian('The Sent folder is unavailable over IMAP. Enable it in Gmail to complete this search.'),
    'La cartella Posta inviata non è disponibile via IMAP. Attivala in Gmail per completare questa ricerca.',
  );
});

test('attachment visual preview messages are translated', () => {
  assert.equal(
    backendTextItalian('Install WordCraft CLI and PdfCraft CLI to preview this attachment.'),
    'Installa WordCraft CLI and PdfCraft CLI per visualizzare questo allegato.',
  );
  assert.equal(backendTextItalian('Choose a page between 1 and 12.'), 'Scegli una pagina tra 1 e 12.');
  assert.equal(
    backendTextItalian('The rendered page exceeds the 16 MiB preview budget.'),
    'La pagina renderizzata supera il limite di anteprima di 16 MiB.',
  );
  assert.equal(
    backendTextItalian('The visual preview is no longer available. Open the attachment again.'),
    "L'anteprima visiva non è più disponibile. Apri di nuovo l'allegato.",
  );
});

test('a saved attachment that Windows security removed is explained in both languages', () => {
  // The Rust literal is REMOVED_BY_WINDOWS_SECURITY in src-tauri/src/zone.rs.
  const removed = 'Windows security removed the saved file. Check Windows Security.';
  assert.equal(
    backendTextItalian(removed),
    'La sicurezza di Windows ha rimosso il file salvato. Controlla Sicurezza di Windows.',
  );
  assert.equal(backendTextEnglish(removed), removed);
});

test('storage messages use the Italian button name and decimal comma', () => {
  assert.equal(
    backendTextItalian('Protected mail and drafts need 1.25 GB before the safety reserve. Choose a larger limit or Unlimited. No data was removed.'),
    'La posta protetta e le bozze richiedono 1,25 GB prima della riserva di sicurezza. Scegli un limite più alto o Illimitato. Nessun dato è stato rimosso.',
  );
});

test('plurals follow the captured count', () => {
  assert.equal(backendTextItalian('1 messages indexed'), '1 messaggio indicizzato');
  assert.equal(backendTextItalian('12 messages indexed'), '12 messaggi indicizzati');
  assert.equal(backendTextItalian('0 messages indexed'), '0 messaggi indicizzati');
  assert.equal(
    backendTextItalian('3 messages indexed; 1 skipped because the account storage limit was reached'),
    "3 messaggi indicizzati; 1 saltato perché è stato raggiunto il limite dell'archivio locale dell'account",
  );
  assert.equal(
    backendTextItalian('1 messages indexed. 2 messages were not indexed because content was unavailable or exceeded the temporary download limit.'),
    '1 messaggio indicizzato. 2 messaggi non sono stati indicizzati perché il contenuto non era disponibile o superava il limite di download temporaneo.',
  );
});

test('wrappers translate the inner backend message and keep third-party text', () => {
  assert.equal(
    backendTextItalian('me@example.com: Reconnect this mail account in Settings'),
    'me@example.com: Riconnetti questo account di posta nelle Impostazioni.',
  );
  assert.equal(
    backendTextItalian('me@example.com: connection reset by peer'),
    'me@example.com: connection reset by peer',
  );
  assert.equal(
    backendTextItalian('Scheduled message ‘Budget’: Add at least one recipient'),
    'Messaggio programmato ‘Budget’: Aggiungi almeno un destinatario.',
  );
  assert.equal(
    backendTextItalian('Sign-in failed. (Google said: AUTHENTICATE failed)'),
    'Accesso non riuscito. (Google ha risposto: AUTHENTICATE failed)',
  );
  assert.equal(
    backendTextItalian('Sync stopped before completion: connection reset. Confirmed downloads are retained; sync again to continue.'),
    'Sincronizzazione interrotta prima del completamento: connection reset. I download confermati sono conservati; sincronizza di nuovo per continuare.',
  );
  assert.equal(
    backendTextItalian('Sync stopped before completion: Gmail message could not be read. Confirmed downloads are retained; sync again to continue.'),
    'Sincronizzazione interrotta prima del completamento: Impossibile leggere il messaggio di Gmail. I download confermati sono conservati; sincronizza di nuovo per continuare.',
  );
});

test('desktop send path and MCP settings messages are translated', () => {
  assert.equal(backendTextItalian('Operation not found'), 'Operazione non trovata.');
  assert.equal(
    backendTextItalian('This draft already has an active or uncertain delivery. Check the outbox.'),
    "Questa bozza ha già un invio attivo o dall'esito incerto. Controlla la posta in uscita.",
  );
  assert.equal(
    backendTextItalian('Delivery was attempted but its result could not be saved. Do not resend. Repository lock failed'),
    "L'invio è stato tentato, ma non è stato possibile salvarne il risultato. Non inviarlo di nuovo. Impossibile bloccare l'archivio.",
  );
  assert.equal(
    backendTextItalian('MCP permissions file is invalid; repair it in Settings'),
    'Il file dei permessi MCP non è valido; correggilo nelle Impostazioni.',
  );
});

test('multi-line input is translated per line and blank lines survive', () => {
  assert.equal(
    backendTextItalian('Server search cancelled\nsomething unknown\n\nme@example.com: Sign-in failed.'),
    'Ricerca sul server annullata.\nsomething unknown\n\nme@example.com: Accesso non riuscito.',
  );
});

test('offline assistant answer translates the template and keeps mail content', () => {
  const answer = backendTextItalian(
    '(Built-in rules: add an AI key in Settings for real answers.) These items match your question:\n\nMessage [7] Budget from maria@example.com: Please review\nkeep this body line\nEvent Standup on Mon 3 Nov: bring notes\n',
  );
  assert.equal(
    answer,
    '(Regole integrate: aggiungi una chiave AI nelle Impostazioni per ottenere vere risposte.) Questi elementi corrispondono alla tua domanda:\n\nMessaggio [7] Budget da maria@example.com: Please review\nkeep this body line\nEvento Standup il Mon 3 Nov: bring notes\n',
  );
});

test('joined index warnings are translated part by part', () => {
  assert.equal(
    backendTextItalian('a.pdf: exceeds the 25 MiB per-attachment memory budget; b.png: no extractable text; content coverage is partial'),
    'a.pdf: supera il limite di memoria di 25 MiB per allegato; b.png: nessun testo estraibile; copertura dei contenuti parziale',
  );
  assert.equal(
    backendTextItalian('Message has more than 1000 MIME parts; remaining parts were skipped; c.pdf: no extractable text; content coverage is partial; a server note'),
    'Il messaggio ha più di 1000 parti MIME; le parti restanti sono state saltate; c.pdf: nessun testo estraibile; copertura dei contenuti parziale; a server note',
  );
  assert.equal(backendTextItalian('plain text; with semicolons; in it'), 'plain text; with semicolons; in it');
});

test('unknown text, Italian text and odd input pass through unchanged', () => {
  for (const text of ['Something new from a server', 'Una registrazione è già in corso.', '', '\n']) {
    assert.equal(backendTextItalian(text), text);
  }
  const long = 'x'.repeat(5000);
  assert.equal(backendTextItalian(long), long);
  assert.equal(backendTextItalian(undefined), undefined);
  assert.equal(backendTextItalian(null), null);
});

// Exact pairs chosen by `if it` in app/src/application/mail.rs.
const LOCALE_PAIRS = [
  ['Risultati locali: parte della posta sul server potrebbe mancare. Cerca sul server per completare questa ricerca.', 'Local results: some mail on the server may be missing. Search the server to complete this search.'],
  ['Risultati parziali: alcuni messaggi o allegati non hanno testo ricercabile disponibile.', 'Partial results: some message or attachment text is unavailable.'],
  ['Server controllato per questa ricerca. I risultati descrivono la posta disponibile al momento del controllo.', 'Server checked for this search. Results reflect mail available at the time of the check.'],
  ['Nel testo', 'In message text'],
  ['Corrisponde ai filtri', 'Matches the filters'],
  ['Allegato: Q4 $1.pdf', 'Attachment: Q4 $1.pdf'],
];

test('locale-chosen Rust pairs round-trip in both directions', () => {
  for (const [italian, english] of LOCALE_PAIRS) {
    assert.equal(backendTextEnglish(italian), english);
    assert.equal(backendTextItalian(english), italian);
    assert.equal(backendTextItalian(backendTextEnglish(italian)), italian);
    assert.equal(backendTextEnglish(backendTextItalian(english)), english);
  }
});

test('Italian exact messages become English and name the English buttons', () => {
  assert.equal(backendTextEnglish('Una registrazione è già in corso.'), 'A recording is already in progress.');
  assert.equal(backendTextEnglish('Formato non supportato.'), 'Unsupported format.');
  assert.equal(backendTextEnglish('Scegli nuovamente il file con Importa audio.'), 'Choose the file again with Import audio.');
  assert.equal(backendTextEnglish('Verbale not found'), 'Summary not found');
  assert.equal(backendTextEnglish('Whisper Tiny · multilingue'), 'Whisper Tiny · multilingual');
  assert.equal(backendTextEnglish('Fuso non riconosciuto: uso UTC / Unknown time zone: using UTC'), 'Unknown time zone: using UTC');
});

test('Italian templates keep captures and translate an inner Italian message', () => {
  assert.equal(
    backendTextEnglish("Il servizio di trascrizione ha risposto con errore 503. L'audio rimane salvato."),
    'The transcription service responded with error 503. The audio stays saved.',
  );
  assert.equal(
    backendTextEnglish("Metadati recuperati dalla copia meeting.bak. L'ultima modifica potrebbe essere incompleta; l'audio viene verificato separatamente."),
    'Metadata recovered from copy meeting.bak. The last change may be incomplete; the audio is checked separately.',
  );
  assert.equal(
    backendTextEnglish("Verifica dell'audio non completata: Indice meeting danneggiato.. I file sono conservati; riprova Recupera audio."),
    'Audio check not completed: Meeting index is damaged. The files are kept; try Recover audio again.',
  );
  assert.equal(
    backendTextEnglish("Registrazione attiva, ma l'indice non è stato aggiornato: disk full. Usa Termina per fermarla."),
    'Recording is active, but the index was not updated: disk full. Use Stop and save to end it.',
  );
});

test('Italian wrappers translate an English inner message in Italian mode', () => {
  assert.equal(
    backendTextItalian('Verifica audio non riuscita: Invalid audio block'),
    'Verifica audio non riuscita: Blocco audio non valido.',
  );
  assert.equal(
    backendTextItalian("Aggiornamento dell'indice non riuscito: Invalid audio block. I blocchi audio già scritti restano recuperabili."),
    "Aggiornamento dell'indice non riuscito: Blocco audio non valido. I blocchi audio già scritti restano recuperabili.",
  );
});

test('English direction handles multi-line answers and keeps meeting content', () => {
  assert.equal(
    backendTextEnglish('Operazione annullata.\nqualcosa di nuovo\n\nMeeting [3] Budget — fonte seg-1 / versione legacy / posizione audio non disponibile ms: rivediamo il budget'),
    'Operation cancelled.\nqualcosa di nuovo\n\nMeeting [3] Budget — source seg-1 / version legacy / audio position unavailable ms: rivediamo il budget',
  );
  assert.equal(
    backendTextEnglish('Meeting [3] Budget — fonte title / versione v2 / posizione audio 1500 ms: testo'),
    'Meeting [3] Budget — source title / version v2 / audio position 1500 ms: testo',
  );
});

test('stored titles, English text and odd input pass through the English direction', () => {
  for (const text of [
    'Riunione del 02/11/2026 10:30',
    'Audio importato',
    'Registrazione recuperata #4',
    'Meeting #4 da verificare',
    'A recording is already in progress.',
    '',
    '\n',
  ]) {
    assert.equal(backendTextEnglish(text), text);
  }
  assert.equal(backendTextEnglish(undefined), undefined);
  assert.equal(backendTextEnglish(null), null);
  const long = 'x'.repeat(5000);
  assert.equal(backendTextEnglish(long), long);
});

test('contacts messages have Italian text', () => {
  const expected = {
    'Choose an address book': 'Scegli una rubrica.',
    'Contact not found': 'Contatto non trovato.',
    'Select at least two cards': 'Seleziona almeno due schede.',
    'Choose at least one address book': 'Scegli almeno una rubrica.',
    'Write permission not granted for this address book': 'Permesso di scrittura non concesso per questa rubrica.',
    'Contacts data is damaged; original file preserved': 'I dati dei contatti sono danneggiati; il file originale è stato conservato.',
    'Conflict: choose which version to keep': 'Conflitto: scegli quale versione tenere.',
    'Cards seen in mail cannot be deleted; hide them instead': 'Le schede viste nella posta non si eliminano: nascondile.',
    'Allow access to Google Contacts for this account first': "Consenti prima l'accesso ai contatti Google per questo account.",
  };
  for (const [english, italian] of Object.entries(expected)) assert.equal(backendTextItalian(english), italian, english);
});

test('the messages of the simple Meetings page are translated to English', () => {
  const expected = {
    'Installa il modello di trascrizione in Impostazioni.': 'Install the transcription model in Settings.',
    'Configura un endpoint compatibile OpenAI in Impostazioni per trascrivere in remoto.': 'Set up an OpenAI-compatible endpoint in Settings to transcribe remotely.',
    "L'audio di questa riunione è stato eliminato dopo la trascrizione.": 'The audio of this meeting was deleted after transcription.',
    "Non c'è audio da eliminare.": 'There is no audio to delete.',
    'Trascrizione non configurata.': 'Transcription is not set up.',
    'Scegli se trascrivere su questo computer o con il servizio remoto.': 'Choose whether to transcribe on this computer or with the remote service.',
    // Already translated for the transcription flow; the Settings validation reuses them.
    'Lingua di trascrizione non valida.': 'Invalid transcription language.',
    'Modello di trascrizione non supportato.': 'Unsupported transcription model.',
  };
  for (const [italian, english] of Object.entries(expected)) assert.equal(backendTextEnglish(italian), english, italian);
});

test('compute server messages are translated', () => {
  assert.equal(
    backendTextItalian('The compute server is not responding; the request will be retried.'),
    'Il server di calcolo non risponde; la richiesta verrà ripetuta.',
  );
  assert.equal(
    backendTextItalian('The compute server returned 255 values, expected 256.'),
    'Il server di calcolo ha restituito 255 valori, attesi 256.',
  );
  assert.equal(backendTextItalian('The semantic search model is still downloading.'), 'Il modello di ricerca semantica è ancora in download.');
});

test('transcription messages of the compute server are translated', () => {
  assert.equal(
    backendTextItalian('The speech recognizer whisper-cli is not installed on the compute server.'),
    'Sul server di calcolo non è installato il riconoscitore vocale whisper-cli.',
  );
  assert.equal(
    backendTextItalian('This server does not offer the transcription model whisper-huge.'),
    'Questo server non offre il modello di trascrizione whisper-huge.',
  );
  assert.equal(
    backendTextEnglish('Scegli se trascrivere con il server di calcolo o con un servizio esterno compatibile OpenAI.'),
    'Choose whether to transcribe with the compute server or an external OpenAI-compatible service.',
  );
});

test('the unpaid subscription message offers to continue without a server (wave 2)', () => {
  assert.equal(
    backendTextItalian('Your Qamille subscription is not active: compute on the server is paused. Subscribe in Settings → Compute, or continue without a server.'),
    'Il tuo abbonamento Qamille non è attivo: il calcolo sul server è in pausa. Abbonati da Impostazioni → Calcolo, oppure continua senza server.',
  );
});

test('compute server texts of the remote mode are translated (wave 3)', () => {
  assert.equal(backendTextItalian('This device is no longer connected to the server'), 'Questo dispositivo non è più collegato al server.');
  assert.equal(
    backendTextItalian('Search by meaning is not available: showing word matches only'),
    'Ricerca per significato non disponibile: risultati solo per parola.',
  );
  assert.equal(
    backendTextItalian('The server uses a different model from the index: reindex to search by meaning again'),
    "Il server usa un modello diverso dall'indice: reindicizza per tornare a cercare per significato.",
  );
  assert.equal(backendTextItalian('Not connected to the Qamille server'), 'Non collegato al server Qamille.');
});

test('the errors of the server assistant are translated, also inside «AI error»', () => {
  assert.equal(backendTextItalian('The assistant is temporarily unavailable.'), "L'assistente è temporaneamente non disponibile.");
  assert.equal(
    backendTextItalian('The assistant model needs more free memory than this server has right now.').startsWith('Il modello'),
    true,
  );
  assert.equal(
    backendTextItalian('The assistant model is being prepared on the server; try again in a few minutes.'),
    "Il modello dell'assistente è in preparazione sul server: riprova tra qualche minuto.",
  );
  assert.equal(
    backendTextItalian('The assistant provider has changed: the consent must be given again.'),
    "Il server ora usa un altro provider per l'assistente: conferma per continuare.",
  );
  assert.equal(backendTextItalian('The conversation is too long for the assistant.'), 'La conversazione è troppo lunga per l’assistente.');
  assert.equal(backendTextItalian('This device is no longer connected to the server.'), 'Questo dispositivo non è più collegato al server.');
  assert.equal(
    backendTextItalian('AI error: This device is no longer connected to the server.'),
    'Errore AI: Questo dispositivo non è più collegato al server.',
  );
});

test('a refused server certificate is told apart from a server that does not answer', () => {
  assert.equal(backendTextItalian('The Qamille server does not answer'), 'Il server Qamille non risponde.');
  assert.equal(
    backendTextItalian("The server's certificate is not trusted").startsWith('Il certificato del server non è considerato attendibile'),
    true,
  );
});

test('a used-up quota and a missing acceptance of the Terms have a fixed Italian sentence', () => {
  assert.match(
    backendTextItalian('The search tokens of your plan are used up for this month: indexing and semantic search on the server are paused until they renew.'),
    /^I token di ricerca del tuo piano sono finiti per questo mese/,
  );
  assert.match(
    backendTextItalian('The transcription minutes of your plan are used up for this month: transcription on the server is paused until they renew.'),
    /^I minuti di trascrizione del tuo piano sono finiti/,
  );
  assert.match(
    backendTextItalian('The assistant tokens of your plan are used up for this month: the assistant on the server is paused until they renew.'),
    /^I token dell'assistente del tuo piano sono finiti/,
  );
  assert.equal(
    backendTextItalian('AI error: The assistant tokens of your plan are used up for this month: the assistant on the server is paused until they renew.').startsWith('Errore AI: I token'),
    true,
  );
  assert.equal(backendTextItalian('Mailbox synchronization is paused: the monthly limit of your plan is used up'), 'La sincronizzazione delle caselle è ferma: il limite mensile del tuo piano è finito.');
  assert.equal(backendTextItalian('The Terms and the privacy notice must be accepted to create the account'), "Per creare l'account vanno accettati i Termini e l'informativa.");
});
