# Qamille: interfaccia ricostruita su Ark UI, direzione "Compatto"

Data: 2026-11-08 · Stato: bozza, da approvare · Base: `main` (`a5e04b9`)

## 1. Intento

L'utente trova l'aspetto attuale "casereccio, sgranato" e vuole "stabilità e sicurezza e coerenza grafica professionale". Come riferimento ha indicato Spark, che gli sembra "moderno e professionale", ma ne vuole lo stile, non tutta la disposizione.

All'inizio aveva chiesto di migrare a React. L'idea è stata scartata per due motivi. React non è né più sicuro né più leggero di Svelte, e l'aspetto dipende dal CSS, non dal framework. Si resta su Svelte 5 e si ricostruisce l'interfaccia "from the ground" su Ark UI.

Tra tre mockup l'utente ha scelto **B · Compatto** ("compatto, sicuramente"; "b è perfetto"). Il mockup è l'artboard `Compatto.dc.html` del canvas https://claude.ai/artifact/4mBYUH2KxirfTt4jZsK7yc.

## 2. Obiettivi

1. Tutte le schermate e tutti i dialog dell'app prendono l'aspetto B: posta, scrittura, calendario, contatti, riunioni, assistente, impostazioni, finestra promemoria.
2. Ogni controllo interattivo ha il comportamento di Ark UI (`@ark-ui/svelte`, motore Zag.js): tastiera, focus, ARIA e chiusura con Esc sono quelli di Ark, non scritti a mano.
3. Esiste una sola fonte per i valori visivi, i token in `desktop/src/ds/tokens/theme.css` (ADR-0009). Esiste una sola fonte per i controlli, le primitive in `desktop/src/ds/primitives/` (ADR-0011).
4. Il testo è nitido. I caratteri sono locali in woff2 e il risultato è controllato con la scala di Windows al 100, 125 e 150%.
5. Ogni schermata si capisce con un colpo d'occhio: non letti, selezione e azione principale si distinguono subito. Quando questo criterio entra in conflitto con la finitura estetica, vince il colpo d'occhio (README del modulo ds).

## 3. Fuori obiettivo

- Migrare a React o a un altro framework.
- Usare Tailwind o un kit con tema già fatto (Skeleton, shadcn-svelte, Park UI).
- Riprodurre logo, icone o marchio di Spark (sono di Readdle).
- Cambiare il backend Rust/Tauri, i comandi IPC o i dati.
- Aggiungere funzioni o nuove scorciatoie da tastiera.
- Cambiare il modello delle conversazioni deciso il 2026-11-07. Restano le righe piatte senza rientro, "a <destinatario>" accanto al mittente, il primo destinatario più "+N" ad altezza fissa e la piegatura per risposta.
- Ridisegnare il contenuto HTML delle email ricevute (`MessageHtml.svelte`, iframe `sandbox`).
- Risincronizzare il canvas Doop e il prototipo in Claude (l'utente ha detto "ignora Doop per adesso"). `npm run ds:export` e `test:ds-export` invece vanno tenuti funzionanti.

## 4. Cosa farebbe fallire il lavoro anche se "funziona"

- Alla consegna resta una schermata con un pezzo nel vecchio aspetto e un pezzo nel nuovo.
- Il testo appare sfocato o sgranato a una delle tre scale.
- Peggiorano la reattività o i tempi già misurati (ADR-0007): diventa rosso un test oggi verde, come `test:refresh`, `test:done-latency` o `test:mail-opening`.
- Entra una dipendenza con versione non fissata, oppure fuori dal gruppo Ark/Zag + `@floating-ui/dom` + `@internationalized/date`.
- Si perde il colpo d'occhio: non letti, selezione e conteggi diventano meno evidenti di oggi.

## 5. Vincoli

- **Framework:** Svelte 5.57.3. `@ark-ui/svelte` 5.25.1 richiede `svelte >= 5.29.0`.
- **CSP** in `desktop/src-tauri/tauri.conf.json`: `font-src 'self'`, `style-src 'self' 'unsafe-inline'`, `connect-src` senza host esterni. I caratteri vanno quindi in `desktop/public/fonts/`, come oggi Lexend e Source Sans 3. Niente Google Fonts a runtime.
- **ADR-0008:** tre contenitori. Pannello laterale non modale (420 e 760 px), dialog modale (420, 560 e 960 px), popover ancorato.
- **ADR-0009:** ogni valore visivo arriva da un token (`tests/style-tokens.mjs`).
- **ADR-0011:** il modulo `src/ds/` non importa da `lib/`, da `components/` né da `@tauri-apps` (INV-001). Le classi dei controlli e `<dialog>` non si scrivono a mano fuori dalle primitive (INV-002, `tests/ds-usage.mjs`).
- **Build:** `npm run build` esegue `check` + `test:ds` + `vite build`. L'exe di rilascio si produce solo con `Build-Qamille.bat /no-pause`, oppure con `--features custom-protocol` (AGENTS.md).
- **Ambiente condiviso:** altre sessioni modificano il repo nello stesso momento. Mai `git stash`. Ogni test usa una porta Vite propria. Il lavoro va fatto in un worktree separato partito da `main`.
- **Dati:** mai avviare l'app sui dati reali dell'utente. Solo account Sample o copie.
- **Scorciatoie esistenti** (in `App.svelte`): `N` nuovo messaggio, `R` rispondi, `E` archivia, `Ctrl+K` cerca, `Ctrl+Z` annulla, frecce su e giù. Le scorciatoie mostrate nel mockup (`/`, `#`, `H`, `T`) non esistono e non vanno aggiunte: i suggerimenti mostrano solo quelle reali.

## 6. Direzione visiva decisa (dal mockup B)

Questi valori sono quelli del mockup scelto. Diventano i token del tema chiaro. Il sorgente del mockup è copiato in `docs/specs/2026-11-08-ark-compatto-mockup.dc.html`. Spazi, padding e altezze delle righe si misurano lì; i valori sono tutti scritti in linea.

| Ruolo | Valore |
|---|---|
| Carattere unico | IBM Plex Sans (OFL), pesi 400/500/600/700 |
| Testo normale | 13,5 px, interlinea 1,4 |
| Titolo della vista | 15 px, peso 600 |
| Oggetto nel lettore | 20 px, peso 600 |
| Corpo del messaggio | 14 px, interlinea 1,6, larghezza massima 760 px |
| Etichette di gruppo ("Oggi", "Ieri") | 11,5 px, peso 600, maiuscolo, fondo `#F6F7F9` |
| Testo principale / corpo / secondario | `#111827` / `#1F2937` / `#4B5563` |
| Bordi: pannelli / righe / campi | `#E3E6EA` / `#EEF0F3` / `#DDE1E7` |
| Accento | `#4F46E5` (indaco), solo per non letti, selezione, azione principale e conteggi |
| Riga selezionata | `#EDEDFC` (accento al 10%) |
| Righe della conversazione aperta | `#F7F8FE` |
| Barra laterale: fondo / voce attiva | `#16181D` / `#2A2E38` |
| Barra laterale: testo / testo spento / etichette | `#FFFFFF` (attiva) / `#D3D7DE` / `#A3AAB7` |
| Icona attiva nella barra laterale | `#A7A3F2` (accento schiarito al 50%) |
| Raggi | 6 px per controlli e riquadri, 4 px per etichette e chip |
| Altezza dei controlli | 32 px (voci della barra laterale, pulsanti, campi) |
| Ombre | nessuna a riposo, si separa con i bordi |
| Icone | tratto 1,8, 16 px nei controlli |

Struttura delle viste, come nel mockup:

- **Barra laterale scura.** Il marchio è il fulmine di Qamille già presente in `Sidebar.svelte`, poi il pulsante per comprimere. "Nuovo messaggio" nel colore di accento con `N`. Account come righe da 32 px con un quadratino colorato. Cartelle con i conteggi: quello della cartella attiva è nel colore di accento. Gli strumenti stanno in fondo, separati da un filetto.
- **Elenco.** In alto il titolo con "N da leggere su M", la ricerca da 32 px e le schede sottolineate (Tutte, Da leggere, Con allegati) con i conteggi. Poi le righe a due linee senza avatar:
  - linea 1: mittente, "a …", allegato, numero di messaggi con la freccia per piegare, ora;
  - linea 2: oggetto in peso 500 o 600, poi "—" e l'anteprima;
  - un messaggio non letto ha un punto di 6 px nel colore di accento, il mittente in peso 700 e l'ora nel colore di accento.
  
  Le risposte di una conversazione aperta sono righe più basse, allineate alle altre senza rientro.

  Le schede restano quelle di oggi (`Smart`, `All`, `Unread` in `MailList.svelte`), con le etichette tradotte che hanno già, insieme al pulsante "Conversazioni". La scheda "Con allegati" del mockup non esiste e non va aggiunta. "N da leggere su M" si mostra solo se il frontend ha già entrambi i numeri; altrimenti resta solo "N da leggere". I quadratini degli account usano `account.color`, che esiste già.
- **Lettore.** Barra in alto con Rispondi, Rispondi a tutti e Inoltra con testo; Archivia, Elimina, Posticipa e Da fare con sola icona; poi "1 di 12" e le frecce. Sotto: l'oggetto con l'etichetta dell'account, un riquadro grigio Da / A / Data, il corpo e gli allegati come chip.

## 7. Le mie scelte (non decise dall'utente)

L'utente ha visto e approvato solo la schermata della posta in tema chiaro. Il resto l'ho deciso io, e va mostrato prima di chiudere il lavoro:

1. **Tema scuro.** Stessa struttura, tutto scuro, con la barra laterale un gradino più scura del contenuto. Testo e controlli con contrasto di almeno 4,5:1. I valori esatti li sceglie chi costruisce.
2. **Resto dell'app.** Calendario, contatti, riunioni, assistente, impostazioni, finestra di scrittura e promemoria prendono la stessa densità, gli stessi token e le stesse primitive. Non sono stati disegnati a parte.
3. **Un solo carattere.** IBM Plex Sans sostituisce sia Source Sans 3 sia Lexend (oggi `--font-text` e `--font-title`). I due token restano, e puntano allo stesso carattere.
4. **Conferma visiva prima del resto.** Una volta finita la posta in entrambi i temi, le schermate vanno mostrate all'utente in screenshot, prima di passare alle altre.
5. **Test.** I test che controllano dettagli del DOM nativo (`<select>`, `<dialog>`, checkbox, `title=`) si riscrivono, perché verifichino lo stesso comportamento attraverso ruolo ed etichetta accessibile. Le verifiche di comportamento e di tempo (latenza, refresh, ordine, esiti) non si toccano. Le immagini di riferimento di `visual-snapshot.mjs` e `visual-compare.mjs` si rigenerano solo dopo che l'utente ha approvato gli screenshot.
6. **Righe dell'elenco senza Ark.** Le righe della posta restano markup Svelte semplice, stilato con i token. Ark serve per i controlli, non per ogni riga, così i tempi misurati da `test:refresh`, `test:done-latency` e `test:mail-opening` non peggiorano.
7. **Livelli.** I livelli che Ark porta fuori dal flusso (portal di popover, dialog, suggerimenti, toast) usano la scala già esistente `--z-popover` < `--z-panel` < `--z-modal` < `--z-toast`. Nessun valore nuovo.
8. **Versioni.** Le dipendenze dirette sono fissate a versione esatta e `package-lock.json` è aggiornato. Niente `overrides` per i pacchetti Zag: il lockfile basta a fissarli.

## 8. Ark: cosa sostituisce cosa

Inventario su `main`: 42 `<select>`, 23 checkbox, 6 campi data, 2 campi ora, 1 cursore, 3 `role="tablist"`, 61 `title=` usati come suggerimento, 7 `<Modal>`, 2 `<SidePanel>`, 1 `<Popover>`.

| Oggi | Con Ark | Dove |
|---|---|---|
| `Modal.svelte` (`<dialog>`) | Dialog | conferme, contatto, unione, guida Google, stampa |
| `SidePanel.svelte` | Dialog non modale o Drawer (sceglie chi costruisce) | dettaglio evento, anteprima allegati |
| `Popover.svelte` | Popover | filtri, menu ancorati |
| `<select>` | Select | impostazioni, calendario, contatti |
| checkbox | Checkbox o Switch (Switch per le impostazioni on/off) | impostazioni, elenchi |
| `role="tablist"` a mano | Tabs o Segment Group | schede dell'elenco, impostazioni |
| `title=` sui pulsanti a sola icona | Tooltip, con la scorciatoia reale quando esiste | ovunque |
| campi data e ora nativi | Date Picker (e Date Input o il campo nativo stilizzato per l'ora, sceglie chi costruisce) | calendario, posticipa |
| cursore nativo | Slider | `MeetingPlayer` |
| `RecipientInput` | Tags Input + Combobox | destinatari nella scrittura |
| notifiche e `UndoStack` | Toast | annulla, esiti |
| piegatura delle conversazioni | Collapsible, se non complica le righe (sceglie chi costruisce) | elenco |

L'editor (Tiptap, `RichEditor.svelte`) e l'anteprima allegati (ArtCraft/PdfCraft) non sono controlli di Ark: cambiano solo nell'aspetto.

Le primitive in `src/ds/primitives/` avvolgono i componenti di Ark e li stilano con i selettori `[data-scope][data-part]`, usando solo token. Il resto dell'app usa solo le primitive.

## 9. Sicurezza delle dipendenze

- `@ark-ui/svelte` fissato alla versione esatta (5.25.1 o successiva, senza `^`). Porta con sé circa 68 pacchetti `@zag-js/*` dello stesso team con versioni fissate, più `@floating-ui/dom` e `@internationalized/date`.
- IBM Plex Sans copiato come file woff2 in `desktop/public/fonts/ibm-plex-sans/`, con `OFL.txt`. Nessuna dipendenza npm per il carattere.
- `package-lock.json` aggiornato. `npm audit` senza vulnerabilità high o critical sui pacchetti nuovi.
- La CSP resta com'è.

## 10. Quando è finito (criteri di accettazione)

1. Ogni schermata e ogni dialog è nell'aspetto B, in tema chiaro e scuro. Nessun residuo del vecchio aspetto.
2. Tutti i controlli della tabella al §8 passano dalle primitive basate su Ark. `tests/ds-usage.mjs` è esteso per fallire se fuori da `src/ds/` compaiono `<select>`, `<dialog>`, checkbox nativi o tablist scritti a mano.
3. `npm run build` passa.
4. Tutti i `test:*` di `desktop/package.json` passano, riscritti dove serve secondo il §7.5. I test `:native` girano solo con account Sample. Il primo passo del lavoro è eseguire tutti i test su `main` e annotare quelli già rossi: è la base di partenza, e quei test non devono peggiorare.
5. Screenshot di ogni schermata a 1440×900 e a 1000×700, nei due temi, più un confronto della nitidezza del testo a scala 100/125/150% tra prima e dopo.
6. Exe di rilascio prodotto con `Build-Qamille.bat /no-pause`. `node tests/release-theme-probe.mjs` restituisce `http://tauri.localhost/`.
7. ADR-0011 aggiornato (Ark dentro le primitive) e README di `src/ds/` riscritto con i nuovi token e le nuove primitive, nello stesso lavoro.

## 11. Cosa resta a chi costruisce

- I valori esatti del tema scuro (§7.1) e i token che mancano al mockup: errore, successo, priorità, calendario.
- Le API delle primitive e il nome di quelle nuove.
- Le scelte segnate "sceglie chi costruisce" al §8.
- L'ordine delle schermate dopo la posta.
- Come adattare la densità di calendario e riunioni (`Calendar.svelte` 1.839 righe, `Meetings.svelte` 2.093) senza cambiarne il funzionamento.

## 12. Stima

Circa 14–20 giorni lavorativi, cioè 3–4 settimane:

- token, caratteri e primitive su Ark: 3–4 giorni;
- posta (barra laterale, elenco, lettore, scrittura): 3–4 giorni;
- calendario: 2–3 giorni;
- riunioni: 2–3 giorni;
- contatti, impostazioni e dialog: 2–3 giorni;
- tema scuro, verifica e rilascio: 2–3 giorni.
