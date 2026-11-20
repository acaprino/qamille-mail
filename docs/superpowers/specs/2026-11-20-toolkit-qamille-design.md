# Qamille sul toolkit condiviso: design

Bozza del 2026-11-20, da approvare. Riguarda tre repository:
- il toolkit, nel repository ufficiale `acaprino/toolkit` (pubblico, MIT, creato il 2026-11-20; checkout locale in `D:\Projects\toolkit`). È l'unica fonte del toolkit: nessun'altra copia conta;
- Ethel, che lo usa oggi;
- Qamille, che lo adotta.

## Intento e motivo

Un solo sistema visivo per le app del proprietario, oggi Ethel e Qamille. Il toolkit `@acaprino/toolkit` porta:
- il comportamento dei controlli (Ark UI);
- la loro forma (le ricette di Park UI);
- il movimento;
- il motore dei temi.

Ogni app porta la propria identità sotto forma di parametri.

Lettura del proprietario, non sua parola: oggi Qamille ha un sistema suo (`desktop/src/ds`) accanto al toolkit, e i due sistemi duplicano primitive, token e test. Una correzione, per esempio di focus, contrasto o tastiera, va fatta due volte, e le due app divergono.

## Decisioni del proprietario (2026-11-20)

- Il toolkit non ha colori propri: al massimo un sistema di theming a parametri, come in Ethel. Fatto: `acaprino/toolkit` non ha temi propri, li passa l'app.
- In Qamille i parametri li fissa Qamille. L'aspetto resta «Compatto» e l'utente sceglie solo chiaro, scuro o sistema.
- Il toolkit supporta più densità.
- Il toolkit diventa pubblico, con licenza MIT.
- I due punti da risolvere sono la densità e il modo in cui Qamille prende il toolkit.
- Si usa solo il repository ufficiale `acaprino/toolkit`.
- **Quando le forme del toolkit e di Qamille sono diverse, vince il toolkit** (strada A). Qamille tiene come parametri i colori «Compatto», il carattere, la densità e i raggi; il resto della forma, del testo e del movimento è quello del toolkit. Le differenze accettate sono elencate sotto, in «Differenze accettate».

## Obiettivi

1. **Repository pubblico con release.** Il toolkit vive in `acaprino/toolkit` con licenza MIT: fatto. Il suo README oggi dice di usarlo da un checkout locale, con `npm ci`, `npm run build` e una dipendenza `file:`. Non ha ancora release.
   - Manca la distribuzione: GitHub Release con il `dist` compilato, un manifest con gli SHA-256 e `provenance.json`, come `artcraft-web`. Così Qamille, la sua CI e chi lo clona non hanno bisogno di un checkout accanto.
2. **Identità tutta a parametri.** Ogni valore d'identità è un parametro dell'app: colori (fatto), raggi, ombre, carattere e densità. Il toolkit tiene solo una base neutra per il primo disegno.
3. **Densità.** Un attributo sulla radice, `data-density`, con tre livelli che riprendono le tre altezze di Park:
   - `compact`: controlli da 32 px;
   - `regular`: 36 px;
   - `comfortable`: 40 px.

   Altezze, spaziature interne, distanze e corpo del testo dei controlli sono token che cambiano con l'attributo. Vale dal primo disegno. Con un puntatore grossolano restano i 44 px, in ogni densità.
4. **Tutto su Ark UI.** Le primitive oggi su Base UI passano ad Ark (dialog, sheet, select, popover, menu, checkbox, tooltip, toggle group, radio group, input), e Base UI esce dalle dipendenze.
5. **Nessuna parola fissa.** Ogni testo visibile o accessibile delle primitive arriva dall'app, come prop o da un dizionario passato una volta.
6. **Qamille sul toolkit.**
   - Qamille prende il toolkit con `npm run toolkit:sync`: ultima release, controllo degli SHA-256 e di `provenance.json`; senza rete resta la copia presente; una variabile `TOOLKIT_DIR` forza un checkout locale.
   - Le primitive di `desktop/src/ds` lasciano il posto a quelle del toolkit. Qamille mantiene i suoi parametri, e cambia solo nelle «Differenze accettate».
7. **Ethel dal repository ufficiale.** Anche Ethel usa `acaprino/toolkit`, senza cambi visibili; come lo prende lo decide chi lavora su Ethel.

## Non obiettivi

- In Qamille la scelta del tema da parte dell'utente: si farà dopo, come lavoro a parte.
- Cambiare l'aspetto di Qamille o di Ethel.
- Portare nel toolkit componenti di prodotto: per Qamille `RecipientsInput`, `NavList` e `Count`, per Ethel `Stack`, `Kanban` e `TaskSheet`. Restano nelle app, costruiti sulle primitive del toolkit.

## Anti-obiettivi (un fallimento anche se funziona)

- Dopo la migrazione Qamille perde i suoi parametri (colori «Compatto», IBM Plex Sans, densità compatta, raggi), oppure cambia in un modo che non sta tra le «Differenze accettate».
- Alla fine della migrazione Qamille ha due fonti per lo stesso token.
- Una build di Qamille chiede un login, un token o un pacchetto privato.
- Il bundle di Qamille contiene due copie di Ark o di Zag.
- A Qamille si applicano le regole di prodotto di Ethel, cioè le R1–R13 di `DESIGN.md` («ADHD first», icone ovunque, niente barra d'accento sulla voce attiva).

## Vincoli

- **Qamille, ADR-0011 v3.**
  - Dipendenze a versione esatta.
  - Ark UI dietro le primitive: i componenti dell'app non importano `@ark-ui/*`.
  - Valori visivi solo da token (ADR-0009).
  - Il passaggio al toolkit va scritto in un **ADR-0011 v4**.
- **Test di Qamille.** Selezionano su `[data-scope][data-part]` di Ark, sulle classi di `system.css`, su ruoli e nomi. Ogni primitiva sostituita deve tenerli validi, oppure aggiornarli nello stesso passo.
- **Aspetto nativo** (`docs/aspetto-nativo.md`):
  - il primo disegno lo decide `public/theme-boot.js`;
  - lo sfondo nativo è `--ground` (`tauri.conf.json` e `DARK_GROUND` in `native_feel.rs`);
  - devono restare verdi `tests/native-feel.mjs`, `tests/forced-colors.mjs`, `tests/contrast.mjs` e `tests/style-tokens.mjs`.
- **CSP di Qamille.** `style-src 'self' 'unsafe-inline'`, `font-src 'self'`: il carattere resta locale, IBM Plex Sans in `public/fonts`.
- **Lingue.** Qamille è in italiano e inglese, quindi le primitive non portano testo inglese fisso.
- **Il toolkit oggi:**
  - intervalli con `^` e versioni diverse da Qamille (`motion` 14.6.0 contro 14.7.1, Tailwind 4.3.5 contro 4.3.3);
  - `dist/index.js` include dentro di sé Ark, Zag e Base UI.

  La release deve fissare versioni esatte, e un solo valore per dipendenza nelle due app. React, Ark e Motion vanno dichiarati come peer dependency esterne al bundle, così ogni app ne ha una copia sola.
- **Ogni modifica al toolkit si fa in `acaprino/toolkit`**, il repository ufficiale. Nessun'altra copia è una fonte.

## Approcci considerati

**Come Qamille prende il toolkit**
- **Scelto: repository pubblico con release e sync**, come `artcraft-web`. Una fonte sola, nessun login, la stessa via per le due app.
- Pacchetto privato su GitHub Packages: scartato, perché Qamille è pubblico e senza token non si compilerebbe.
- Copia dentro Qamille: scartata, perché le due copie divergono e il codice diventerebbe pubblico comunque.

**Densità**
- **Scelto: attributo sulla radice e token CSS.** Stesso meccanismo del motore dei temi; vale dal primo disegno; nessun nuovo render; anche l'HTML scritto a mano segue.
- Contesto React con varianti di misura: scartato, perché dipende da JavaScript e il primo disegno sarebbe sbagliato.

**Migrazione di Qamille**
- **Proposto: uno strato di compatibilità, poi le primitive una per una.**
  - I token di Qamille (`--ground`, `--accent`, `--muted`, `--hover`…) diventano alias dei token del toolkit, scritti dai parametri «Compatto».
  - Le primitive passano una alla volta, ognuna con i suoi test.
  - Alla fine gli alias e `desktop/src/ds` spariscono.
- Tutto in una volta: scartato, perché romperebbe insieme quasi tutti i test di interfaccia, senza un punto intermedio verificabile.

## Il come già deciso

- **Licenza e nome.** Licenza MIT; repository `acaprino/toolkit`, pacchetto `@acaprino/toolkit`. Sono già così.
- **Le regole R1–R13 di `DESIGN.md` sono di Ethel.** Passano nel documento di design di `@ethel/ui`. Nel toolkit restano solo le regole di meccanismo:
  - soglie di contrasto 4,5:1 e 3:1;
  - anello di focus visibile su ogni controllo;
  - 44 px con un puntatore grossolano;
  - movimento solo dopo un gesto, spento con la riduzione delle animazioni;
  - nessun colore fuori dal motore.

  Mia proposta, coerente con «i parametri li fissa l'app».
- **Il tema di Qamille è una definizione passata al motore.** I valori di partenza sono quelli di «Compatto» in `desktop/src/ds/tokens/theme.css`:
  - accento indaco `#4f46e5`;
  - barra laterale scura (tipo `scura`);
  - IBM Plex Sans;
  - raggi 4/6/8/12 px;
  - nessuna ombra a riposo;
  - densità `compact`.

  Se il motore corregge un colore per rispettare il contrasto, vince il contrasto, e la differenza va annotata.

## Differenze accettate (strada A)

Con la migrazione Qamille cambia in questi punti, e solo in questi:

- **Toast:** la colonna del toolkit, fino a dieci, con il fondo invertito, al posto di un avviso chiaro alla volta. Risolve anche F18 della review nativa: un errore non viene più sostituito.
- **Superfici rialzate:** menu, popover, dialoghi e schede hanno l'ombra di Park al posto del bordo; i pulsanti secondari non hanno bordo.
- **Scala del testo:** quella del toolkit (12, 13 e 14 px per etichette, date e testo, invece di 11,5, 12,5 e 13,5). I titoli più grandi di Qamille, fino a 26 px, diventano gradini in più della scala del toolkit, se mancano: gradini senza valori di un'app.
- **Movimento:** le transizioni brevi del toolkit al posto delle molle. Interruttore, schede e la cifra animata dei contatori usano il movimento del toolkit, e le copie di unlumen in `desktop/src/ds/unlumen/` spariscono.
- **Contrasto:** in modalità chiara i bordi dei campi, dei select e dei pulsanti con contorno arrivano a 3:1, quindi sono più scuri. Oggi sono a 1,3:1, sotto la soglia WCAG 1.4.11, e la review nativa l'aveva segnalato.

## Scelte del costruttore già fatte (proposte, correggibili)

- **Come si verifica.**
  - Ogni primitiva migrata si confronta prima e dopo con `tests/visual-compare.mjs`: le differenze devono stare tutte nell'elenco qui sopra.
  - `tests/tokens-compatto.mjs` controlla i parametri «Compatto» passati al motore, non più ogni esadecimale derivato.
- **Nomi dei token.**
  - Il codice di Qamille passa ai nomi del toolkit per tutto ciò che il toolkit possiede.
  - I token che sono solo di Qamille restano in un file di Qamille: priorità, colori dei mittenti e delle etichette, larghezze dei pannelli, livelli. Non è una seconda fonte, perché il toolkit non li definisce.
  - Qamille adotta gli attributi del toolkit: `data-scheme` per chiaro e scuro, `data-theme` per il tema. Si aggiornano `theme-boot.js`, `message-html.ts` e i test che leggono `data-theme`.
- **Densità e `size`.** La densità sposta la scala intera, e `size` resta relativo alla densità:

  | Densità | Predefinito | `sm` | `xs` |
  |---|---|---|---|
  | `comfortable` (Ethel, nessun cambio) | 40 px | 36 px | 32 px |
  | `regular` | 36 px | 32 px | 32 px |
  | `compact` (Qamille) | 32 px | 32 px | 32 px |

  Con un puntatore grossolano: 44 px.
- **CSS.**
  - Qamille compila Tailwind da sé, con `@source` sul toolkit sincronizzato, e importa `theme.css` e `fonts.css` senza preflight, come fa oggi.
  - La release contiene quindi, oltre a `dist`, i sorgenti che servono a `@source` e i fogli di `src/styles`.
- **Versioni.**
  - `toolkit:sync` prende l'ultima release, senza pin, come per `artcraft-web`.
  - Il toolkit dichiara React, Ark e Motion come peer dependency a versione esatta. Se non coincidono con quelle di Qamille, `toolkit:sync` si ferma con un errore, così un aggiornamento di quelle librerie in Qamille è sempre voluto.
  - I cambi visibili li fermano i test visivi di Qamille.

## Lasciato al costruttore

- I nomi dei token di densità e di ombra.
- Come si mappano i valori di «Compatto» sui parametri del motore.
- L'ordine di migrazione delle primitive.
- La forma del dizionario delle parole.
- Come adattare i singoli test.
- La configurazione della CI del nuovo repository.

## Sotto-progetti, in ordine

Ognuno ha il suo piano, scritto quando ci si arriva.

1. **Release del toolkit.** Il repository pubblico c'è già (`acaprino/toolkit`, MIT, con una CI in `.github/workflows/ci.yml`).
   - Manca un workflow di release che, a ogni rilascio, pubblichi `dist`, il manifest SHA-256 e `provenance.json`.
2. **Parametri e densità.** Raggi, ombre e carattere come parametri della definizione del tema; `data-density` con i tre livelli; peer dependency esterne e versioni esatte.
3. **Primitive per Qamille.**
   - Le primitive restanti passano da Base UI ad Ark.
   - Arrivano quelle che mancano a Qamille: Switch, Tabs, Slider, Menu e Field.
   - Le parole diventano parametri.
4. **Qamille.**
   - `toolkit:sync` e il tema «Compatto» come parametri.
   - Gli alias dei token, poi le primitive sostituite una per una con i test.
   - ADR-0011 v4 e `docs/aspetto-nativo.md` aggiornati.
   - Alla fine `desktop/src/ds` sparisce.
