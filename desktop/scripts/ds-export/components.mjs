// What the design system page says about each component, and how its card is laid out.
// `{w}` in a description is replaced by the captured width of the component, so a measure cannot go stale.

// [card summary, usage text, fallback source path].
export const INFO = {
  // Primitives of the module (desktop/src/ds/primitives). The Ark ones draw their parts from data-scope and data-part (styles/ark.css).
  Button: ['Pulsante dell’app, in quattro varianti: primary, secondary, icon e text.', 'Primary per l’azione principale di una zona, al massimo uno per gruppo. Secondary per le alternative. Icon, quadrato di `control-height` (32×32px) con la sola icona, per azioni ripetute e note dal contesto: richiede sempre `aria-label`. Text per le azioni minori in linea. Con `tip` (e `shortcut`) il pulsante mostra un suggerimento: solo per i pulsanti con la sola icona e solo con scorciatoie vere. Hover, focus e disattivato vengono dalle regole di `system.css`.'],
  Field: ['Etichetta e controllo di modulo.', 'Il testo dell’etichetta e il controllo (`input`, `textarea`) sono i figli. Bordo, raggio, riempimento e colore del controllo vengono dalla regola base: i componenti cambiano solo la larghezza. Una scelta da un elenco è un `Select`, non un `<select>`. Il testo di aiuto è un `p.field-help` accanto al campo.'],
  Icon: ['SVG a contorno da un elenco di tracciati, griglia 24×24.', 'Stesso tratto ovunque: 1,7, estremità e angoli arrotondati, `currentColor`. Un nome sconosciuto ricade sull’icona `mail`. Le dimensioni più usate sono 16 e 18px, e 14, 15 e 17 nei controlli compatti.'],
  Modal: ['Dialog centrato per decidere prima di continuare (ADR-0008), su Dialog di Ark UI.', 'Modale, aperto appena compare e disegnato sopra tutto (spostato in `<body>`, livello `z-modal`): un solo velo `scrim`, bordo `border`, `radius-xl`, `shadow`. Larghezze `modal-sm`, predefinita `modal-md`, `modal-lg`. Ark gestisce focus ed Esc; Esc e il clic sul velo arrivano come `onclose`. Le azioni stanno in un `.modal-actions`. L’anteprima mostra il contenuto di ciascuna misura.'],
  SidePanel: ['Pannello laterale per guardare o modificare una cosa senza perdere il contesto (ADR-0008), su Dialog di Ark UI non modale.', 'Entra da destra in `motion-panel` sopra `scrim-soft`, a `z-panel`; il resto della pagina resta raggiungibile. Esc chiude, e il velo chiude con un clic. Larghezza `panel-wide`, o `panel-narrow` con `narrow`. Barra con titolo e pulsante di chiusura, poi corpo che scorre. Riempie il suo antenato posizionato.'],
  Popover: ['Menu o filtro ancorato al suo pulsante (ADR-0008), su Popover di Ark UI.', 'Per azioni secondarie e filtri. Si apre dal pulsante; Esc, un clic fuori o il focus che esce lo chiudono e il focus torna al pulsante. Esiste solo da aperto, spostato in `<body>`, a `z-popover`. Le voci sono `<button>` semplici. Non usarlo per contenuti lunghi: per quelli apri un pannello o un dialog. L’anteprima mostra il pulsante e il menu aperto.'],
  Tooltip: ['Suggerimento di un controllo senza spazio per le parole, su Tooltip di Ark UI.', 'Nomina l’azione e, se c’è, la sua scorciatoia in un `<kbd>`: è un suggerimento, mai il nome del controllo. Si apre subito col focus da tastiera e dopo circa 400 ms sul passaggio del mouse; Esc, il focus che se ne va e il puntatore che esce lo chiudono. Fondo `sidebar`, `radius-sm`, `z-toast`. Di norma lo scrive `Button` con `tip`.'],
  Select: ['Scelta da un elenco breve, su Select di Ark UI.', 'Un pulsante alto `control-height` con la voce scelta, e un elenco in `<body>` a `z-toast`, usabile anche dentro `Modal` e `SidePanel`. `value` vuoto mostra il `placeholder` in `muted`. Sostituisce ogni `<select>`. L’anteprima mostra l’elenco aperto.'],
  Checkbox: ['Casella con la sua etichetta, su Checkbox di Ark UI.', 'Un clic su tutta la riga la inverte. Quadrato di 16px con `radius-sm`, in `accent` da spuntata; `indeterminate` disegna il trattino di «alcuni, non tutti». Sostituisce ogni `<input type="checkbox">` fuori dalle righe della posta.'],
  Switch: ['Interruttore acceso/spento con la sua etichetta, su Switch di Ark UI.', 'Per le impostazioni che valgono subito; per una scelta che si conferma altrove usa `Checkbox`. Binario di 32×18px, in `accent` da acceso; l’input nascosto ha `role="switch"`.'],
  Tabs: ['Fila di schede di un elenco, su Tabs di Ark UI.', 'Disegna solo la lista; il contenuto sotto resta a chi la usa. `underline` ha 2px di `accent` sotto la scelta e il conteggio in `muted`; `segment` è il chip rialzato delle impostazioni. Frecce, Home e Fine muovono e scelgono. Sostituisce ogni `role="tablist"` scritto a mano.'],
  Slider: ['Cursore a un solo valore, su Slider di Ark UI.', 'Traccia di 4px, tratto in `accent`, pomello di 16px con l’anello del focus. Frecce, PagSu e PagGiù, Home e Fine muovono il valore senza mai farlo uscire da `min` e `max`. Usato dal lettore dei meeting.'],
  DateField: ['Campo data con calendario, su DatePicker di Ark UI.', 'Una casella che accetta una data scritta come la scrive `locale` e un pulsante che apre il calendario, alto `control-height`. `value` è `YYYY-MM-DD` o vuoto; `min` e `max` limitano i giorni; `translations` dà i nomi dei pulsanti nella lingua dell’app. Il calendario resta dentro il campo, quindi dentro il `Modal` o il `SidePanel` che lo contiene. L’anteprima mostra il calendario aperto.'],
  RecipientsInput: ['Destinatari di un messaggio: chip e suggerimenti dalla rubrica, su TagsInput e Combobox di Ark UI.', 'Un solo campo di testo. Invio, virgola o punto e virgola trasformano il testo in chip; incollare un elenco lo spezza; Backspace su campo vuoto toglie l’ultimo chip. `invalid` segna i chip il cui indirizzo è rifiutato. I suggerimenti stanno in un elenco in `<body>` a `z-toast`. L’anteprima mostra il campo con una ricerca in corso.'],
  Toaster: ['Avvisi dell’app, su Toast di Ark UI.', 'Un avviso alla volta, in basso al centro, a `z-toast`: uno stato per gli esiti e un allarme per gli errori, con il pulsante di chiusura. Si chiude da solo dopo 4,5 secondi (12 per un errore) e aspetta mentre il puntatore o il focus ci sono sopra. Si mostra con `notify(messaggio, errore)`.'],
  // Classes of the system that are not Svelte components, captured from the running app.
  Avatar: ['Iniziali in un quadrato, per gli account e l’utente della barra laterale.', 'Account: 18px, sul colore dell’account mescolato a `sidebar`, iniziale in `sidebar-text-active`; «tutti gli account» porta un’icona su `sidebar-active`. Utente: 24px su `sidebar-active`.', 'desktop/src/styles.css'],
  SearchChip: ['Filtri rapidi nella barra di ricerca.', 'Aprono i campi del filtro corrispondente; lo stato attivo usa `selection`.', 'desktop/src/styles.css'],
  SettingRow: ['Riga di impostazione con etichetta, aiuto e controllo.', 'Una riga per opzione, nell’ordine in cui si legge la pagina.', 'desktop/src/styles.css'],
  // Components of the app, captured from the running preview with sample data.
  Sidebar: ['Barra laterale con composizione, account, cartelle, spazio di lavoro ed etichette.', 'Larga {w}px e comprimibile. Colonna scura (`sidebar`) con voci alte `control-height`: la voce attiva sta su `sidebar-active` e il suo conteggio è in `accent-on-sidebar`.'],
  SearchBar: ['Barra di ricerca con filtri e guida alla sintassi.', 'Sta sopra l’elenco posta; i filtri applicati diventano chip.'],
  MailList: ['Elenco dei messaggi raggruppati per priorità e stato.', 'Largo {w}px. Vista per email o per conversazione, con le schede Intelligente, Tutti e Non letti e i messaggi fissati. Nella vista intelligente le righe stanno in gruppi (Priorità, Nuovi, Notifiche, Newsletter, Fissati, Già letti) con il conteggio; i gruppi minori partono chiusi e ricordano i non letti con un pallino e il conteggio in colore accento.'],
  Reader: ['Lettura del messaggio con azioni, riassunto AI e allegati.', 'Occupa tutto lo spazio a destra dell’elenco. Rispondi è l’unico pulsante pieno; Rispondi a tutti e Inoltra sono secondari; Archivia, Cestino, Posticipa, Fatto, Fissa e le altre azioni sono pulsanti icona con suggerimento e scorciatoia.'],
  MessageHtml: ['Corpo di un messaggio HTML, isolato dal resto dell’app.', 'Nell’app il contenuto gira in un `iframe` sandbox senza accesso alla rete: le immagini remote sono bloccate. Qui è mostrato come blocco semplice, perché le anteprime non ammettono `iframe`.'],
  ThreadReplies: ['Risposte di una conversazione, una riga ciascuna nella lista.', 'Nessun rientro: il legame sta scritto accanto al nome («a Maria Rossi», «a te»). Una risposta che ne ha altre sotto si può chiudere; il pallino blu dei non letti sta sempre nella stessa colonna.'],
  RecipientLine: ['Destinatari «A» o «Cc» del messaggio: tre nomi e «e altri N».', 'Aprendo «e altri N» compare l’elenco completo, in un riquadro alto al massimo `recipients-max` che scorre da solo. «A» e «Cc» stanno su righe separate.'],
  UndoStack: ['Pila di notifiche con annullamento a tempo.', 'Compare dopo azioni reversibili come Archivia, con il tempo rimanente.'],
  AttachmentPanel: ['Pannello laterale che mostra un allegato.', 'Non modale, sopra `scrim-soft`, largo `panel-wide`.'],
  AttachmentPreview: ['Anteprima del contenuto di un allegato.', 'Usata dentro il pannello degli allegati. PDF e altri formati sono resi da motori WebAssembly incorporati.'],
  PrintPreview: ['Dialog di anteprima di stampa di un messaggio.', 'Usa i token `paper`, che non cambiano con il tema.'],
  AddFromMail: ['Dialog che crea un contatto dal mittente del messaggio.', 'Precompila i campi di `ContactForm` con i dati del mittente.'],
  ComposerDock: ['Barra in basso con le schede delle bozze aperte.', 'Una scheda per bozza, più il pulsante per aprirne una nuova.'],
  Composer: ['Finestra di composizione agganciata in basso a destra.', 'Destinatari, oggetto, editor, allegati, firma e invio programmato. Più bozze convivono come schede.'],
  RichEditor: ['Editor di testo ricco basato su TipTap, con barra di formattazione.', 'Usalo per il corpo dei messaggi.'],
  RecipientInput: ['Campo destinatari del compositore e dei contatti, con suggerimenti dalla rubrica, su `RecipientsInput`.', 'Accetta indirizzi separati da virgola o punto e virgola e propone contatti mentre si digita; i chip il cui indirizzo non è valido sono segnati.'],
  Calendar: ['Calendario con viste giorno, settimana e mese.', 'Colonna con i calendari a sinistra, griglia a destra. Gli eventi si aprono nel pannello laterale.'],
  Meetings: ['Pagina dei meeting: elenco in quattro gruppi, ricerca, sintesi e trascrizione.', 'Registra ora usa microfono e audio del computer; le scelte di trascrizione e sintesi stanno in Impostazioni.'],
  Assistant: ['Pagina dell’assistente con domande suggerite e campo di richiesta.', 'Nell’anteprima del browser la risposta non è disponibile: serve l’app desktop.'],
  Contacts: ['Pagina della rubrica: filtri, elenco e dettaglio del contatto.', 'Contatti multi-rubrica con importazione ed esportazione `.vcf`.'],
  ContactForm: ['Dialog per creare o modificare un contatto.', 'Usa `ContactFieldsEditor` per email, telefoni e indirizzi.'],
  ContactFieldsEditor: ['Campi ripetibili del contatto: email, telefoni, indirizzi.', 'Ogni riga ha etichetta, valore e pulsante di rimozione.'],
  DuplicatesPanel: ['Pannello con i gruppi di possibili contatti duplicati.', 'Per ogni gruppo: Stessa persona, Unisci, Persone diverse.'],
  MergeDialog: ['Dialog che unisce più contatti in uno.', 'Mostra il risultato e dove applicarlo: in una sola rubrica o in tutte.'],
  Settings: ['Pagina delle impostazioni con navigazione a sezioni.', 'Sezioni: Generali, Account, Archivio locale, Assistente, Firme, Ricerca, Rubrica, Accesso agenti.'],
  AccountStorage: ['Sezione Archivio locale: limiti e spazio usato per account.', 'Dentro le impostazioni.'],
  IndexingSettings: ['Sezione Ricerca: stato dell’indicizzazione per account.', 'Dentro le impostazioni; permette di mettere in pausa.'],
  ContactsSettings: ['Sezione Rubrica: rubriche, priorità e sincronizzazione.', 'Dentro le impostazioni.'],
  McpSettings: ['Sezione Accesso agenti: server MCP e permessi degli assistenti.', 'Dentro le impostazioni.'],
  AssistantInstallers: ['Installatori che collegano Qamille agli assistenti esterni.', 'Dentro Accesso agenti; mostra i file prima di installare.'],
  GoogleSetupGuide: ['Dialog guida per attivare l’accesso Google.', 'Passi numerati con link esterni alla console Google.'],
  GoogleSignInDialog: ['Dialog per inserire le credenziali OAuth e accedere con Google.', 'Piccolo, `modal-sm`. L’anteprima mostra il suo stato di errore, in inglese, perché il browser non può aprire l’accesso: il messaggio esiste solo nella modalità anteprima.'],
  UnsavedDialog: ['Dialog che chiede cosa fare delle modifiche non salvate.', 'Compare quando si lascia una pagina con modifiche in sospeso. Piccolo, `modal-sm`.'],
  MeetingRecordingIndicator: ['Indicatore fluttuante della registrazione in corso.', 'Resta visibile nelle altre pagine e riapre il meeting al clic. Mostra stato, durata e quanto è già salvato su disco.'],
  ActivityIndicator: ['Indicatore unico delle attività in background.', 'Fase e avanzamento della sincronizzazione stanno solo qui; gli errori restano avvisi separati.'],
  WindowControls: ['Pulsanti della finestra: riduci, ingrandisci, chiudi.', 'Solo nell’app desktop. Il pulsante di chiusura usa `window-close` in hover.'],
  Reminder: ['Promemoria di una riunione imminente, in una finestra propria a forma di pillola.', 'Azioni: Entra, Entra e registra, Chiudi. Reso qui con dati di esempio e il bridge desktop simulato.'],
  AttachedMessageView: ['Messaggio allegato (`.eml`, PEC) aperto nel pannello laterale.', 'Mostra il messaggio formattato e i suoi allegati, con un modo per tornare indietro. Reso qui dalla pagina di test dell’app.'],
};

// A note per prop, shown in "Cosa fornire" after the prop's name.
export const PROP_NOTES = {
  Button: {
    variant: '`primary`, `secondary`, `icon` o `text`: sceglie la classe `primary`, `secondary`, `icon-button` o `text-button`',
    class: 'classi di pagina da aggiungere',
    element: 'il nodo `<button>` (`bind:element`)',
    tip: 'il testo del suggerimento: il pulsante diventa l’attivatore di un `Tooltip`; è un suggerimento, non il nome, quindi serve ancora `aria-label`',
    shortcut: 'la scorciatoia mostrata nel suggerimento; solo una che l’app ha già',
    children: 'il contenuto: testo, o un `Icon`',
  },
  Field: { class: 'classi di pagina da aggiungere', children: 'il testo dell’etichetta e il controllo' },
  Icon: { name: 'il nome del tracciato; uno sconosciuto ricade su `mail`', size: 'lato in px', stroke: 'spessore del tratto' },
  Modal: {
    size: '`sm` o `lg`; senza, la larghezza è `modal-md`',
    onclose: 'chiamata da Esc e dal clic sul velo; chi usa il `Modal` lo smonta',
    'aria-label': 'il nome accessibile del dialog',
    class: 'classi di pagina da aggiungere al contenuto',
    children: 'il contenuto del dialog',
  },
  SidePanel: {
    narrow: 'larghezza `panel-narrow` invece di `panel-wide`',
    element: 'il nodo del pannello (`bind:element`)',
    onscrim: 'chiamata al clic sul velo (e da Esc se manca `onclose`)',
    onopen: 'chiamata quando il pannello è visibile, per mettere il focus',
    onclose: 'chiamata da Esc',
    'aria-label': 'il nome accessibile del pannello',
    children: 'barra e corpo del pannello',
  },
  Popover: {
    open: 'aperto o chiuso (`bind:open`); una voce che fa il suo lavoro lo mette a `false`',
    trigger: 'una snippet che disegna il pulsante e mette `{...props}` su quell’unico elemento',
    label: 'il nome accessibile della superficie',
    class: 'classi di pagina da aggiungere',
    children: 'le voci del menu',
  },
  Tooltip: { text: 'il testo del suggerimento', shortcut: 'la scorciatoia, in un `<kbd>`', children: 'una snippet che riceve `props` e li mette sull’attivatore' },
  Select: {
    value: 'il valore scelto (`bind:value`); una stringa vuota vuol dire nessuna scelta',
    items: 'le voci: `{ value, label, disabled? }`',
    label: 'l’etichetta visibile e il nome del pulsante',
    placeholder: 'il testo in `muted` finché non c’è una scelta',
    'aria-label': 'il nome, quando manca `label`',
    onchange: 'riceve il nuovo valore',
  },
  Checkbox: {
    checked: 'spuntata o no (`bind:checked`)',
    indeterminate: 'disegna il trattino di «alcuni, non tutti»',
    'aria-label': 'il nome, quando non c’è un’etichetta visibile',
    onchange: 'riceve il nuovo valore',
    children: 'l’etichetta',
  },
  Switch: { checked: 'acceso o no (`bind:checked`)', onchange: 'riceve il nuovo valore', children: 'l’etichetta' },
  Tabs: {
    value: 'la scheda scelta (`bind:value`)',
    items: 'le schede: `{ value, label, count?, class?, props? }`',
    variant: '`underline` (predefinita), `segment` o `plain`',
    'aria-label': 'il nome della lista di schede (obbligatorio)',
    activation: '`manual`: le frecce muovono e Invio o Spazio sceglie',
    ids: 'gli id di scheda e pannello per valore, se chi usa disegna i pannelli',
    onpick: 'sente ogni clic, anche sulla scheda già scelta',
    tab: 'disegna l’interno della scheda',
    after: 'disegna ciò che le sta accanto, per esempio un pulsante di chiusura',
  },
  Slider: { value: 'il valore (`bind:value`)', min: 'il minimo', max: 'il massimo', step: 'il passo', 'aria-label': 'il nome (obbligatorio)', onchange: 'riceve ogni valore' },
  DateField: {
    value: '`YYYY-MM-DD`, o vuoto per nessuna data (`bind:value`)',
    'aria-label': 'il nome della casella (obbligatorio)',
    min: 'il primo giorno ammesso',
    max: 'l’ultimo giorno ammesso',
    locale: 'come si scrive la data: giorno per primo per `it-IT`',
    translations: 'i nomi dei pulsanti del calendario, nella lingua dell’app; senza, in inglese',
    disabled: 'blocca la casella e il pulsante',
  },
  RecipientsInput: {
    value: 'l’elenco delle voci, indirizzo o «Nome <indirizzo>» (`bind:value`)',
    suggestions: 'la risposta a `onquery`: `{ email, name? }[]`',
    'aria-label': 'il nome del campo (obbligatorio)',
    onquery: 'chiamata con il testo scritto, per chiedere i suggerimenti',
    invalid: 'dice se un indirizzo è rifiutato: il chip prende `data-invalid`',
    pending: 'il testo non ancora diventato chip (`bind:pending`)',
    input: 'il nodo `<input>` (`bind:input`)',
    translations: 'le parole dei chip, nella lingua dell’app',
    listTranslations: 'le parole dell’elenco dei suggerimenti',
    'suggestions-label': 'il nome dell’elenco dei suggerimenti',
  },
  Toaster: { label: 'il nome della zona degli avvisi', closeLabel: 'il nome del pulsante di chiusura' },
};

// Padding per card. 16: sections with no padding of their own. 24: dialogs and floating surfaces, sized by
// calc(100vw - 48px) and calc(100vh - 48px), which then resolve to the same size as in the 1440x900 window.
export const PADS = {
  AccountStorage: 16, IndexingSettings: 16, ContactsSettings: 16, McpSettings: 16, AssistantInstallers: 16,
  ContactFieldsEditor: 16, RecipientInput: 16, RichEditor: 16, AttachmentPreview: 16,
  UnsavedDialog: 24, GoogleSignInDialog: 24, GoogleSetupGuide: 24, ContactForm: 24, AddFromMail: 24, MergeDialog: 24,
  PrintPreview: 24, UndoStack: 24, MeetingRecordingIndicator: 24, ActivityIndicator: 24, Modal: 24,
};
// Cards whose layout depends on window breakpoints: they lay out at the real window width, content centered.
export const WIDE = new Set(['AttachmentPanel', 'Settings', 'Meetings', 'Reader', 'Assistant', 'Sidebar', 'AttachedMessageView', 'SidePanel']);
// Small cards whose labels hide below the largest breakpoint (max-width:1200px).
export const ABOVE_BREAKPOINTS = new Set(['Button', 'SearchChip']);
// Cards of several parts that need a wider wrap than the default 640px.
export const WRAP = { Button: 1000, Modal: 1024, SidePanel: 1440, Select: 720, DateField: 720, Checkbox: 720, Switch: 720, Tabs: 720, Field: 720, Popover: 480, Tooltip: 480, RecipientsInput: 480, Toaster: 720 };
