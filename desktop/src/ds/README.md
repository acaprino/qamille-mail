# Il sistema visivo di Qamille

Qamille riunisce posta, calendario, rubrica e meeting in un’unica app desktop. L’interfaccia è fatta per chi legge e scrive tutto il giorno, nella direzione «Compatto»: una barra laterale scura, un solo foglio chiaro che porta il lavoro, un solo accento indaco per agire, un solo carattere (IBM Plex Sans), controlli alti 32px, filetti sottili e nessuna ombra a riposo. I componenti e il loro movimento a molla vengono da unlumen UI (ui.unlumen.com), ma leggono i colori, i raggi e le ombre di «Compatto», e il movimento si spegne con `prefers-reduced-motion`. Lo scopo dichiarato è capire una schermata con un colpo d’occhio; quando questo confligge con la finitura estetica, vince il colpo d’occhio. Quanto di questo aspetto sia efficace per qualcuno in particolare è una domanda aperta: vedi «Evidenza e limiti» in fondo.

L’interfaccia è in React 19 e TypeScript. I controlli interattivi hanno il comportamento di Ark UI (`@ark-ui/react`, motore Zag.js): tastiera, focus, ARIA e chiusura con Esc sono quelli di Ark. L’app non lo usa mai direttamente: lo avvolgono le primitive di questo modulo, e l’aspetto delle parti di Ark è in `styles/ark.css` e `styles/ark/*.css`. Alcuni componenti di unlumen (Apple Switch, Animate Digits, Sidebar 001, Animated List) sono copiati in `unlumen/` e stanno sotto le primitive; Tailwind CSS v4, senza preflight e senza la sua tavolozza, legge gli stessi token. unlumen porta i componenti e il movimento, non i colori (ADR-0011 v3, decisione 4).

Questa guida sta nel modulo e si versiona con lui (ADR-0011 v3): una modifica a un token, a una regola di sistema o a una primitiva aggiorna questa pagina nello stesso commit.

## Dove sta

Il sistema visivo è un modulo interno, `desktop/src/ds/`, non un pacchetto. L’app è il solo consumatore.

| Percorso | Cosa contiene |
|---|---|
| `tokens/theme.css` | Tutti i token, nei due temi, con i valori di «Compatto»; in più `--accent-pro` e `--ring`, i nomi che leggono i componenti di unlumen, con lo stesso indaco di `--accent`. È la sola fonte dei valori. |
| `styles/system.css` | Base (corpo, titoli, campi, focus) e regole di sistema: pulsanti e campo. Importa `theme.css`, `ark.css` e `tailwind.css` e dichiara il carattere IBM Plex Sans. |
| `styles/tailwind.css` | Tailwind v4 (tema e utility, niente preflight) e `tw-animate-css`, con il tema, i caratteri e le misure di Sigrid e i valori di Qamille per loro (`src/theme/compatto.css`, generato). Vedi «Tailwind». |
| `styles/ark.css` | In testa gli `@import` di `styles/nav.css` e di `styles/ark/*.css`, poi le regole del suggerimento. Importato da `system.css`. Vedi «Le regole di `ark.css`». |
| `styles/nav.css` | Le regole proprie di `NavList` e `Count`: `.nav-hover`, `.nav-active-bar`, `.count-digits`. |
| `styles/ark/*.css` | L’aspetto delle parti delle primitive, un file per primitiva: `dialog`, `popover`, `menu`, `select`, `checkbox`, `switch`, `tabs`, `slider`, `date-picker`, `recipients`, `toast`. |
| `primitives/` | I componenti React (`.tsx`): `Button`, `Field`, `Modal`, `SidePanel`, `Popover`, `Menu`, `Tooltip`, `Select`, `Checkbox`, `Switch`, `Tabs`, `Slider`, `DateField`, `RecipientsInput`, `Toaster`, `Icon`, `Count`, `NavList`. |
| `unlumen/` | I componenti di unlumen UI copiati nel progetto, con il loro `README.md` che annota le modifiche. Vedi «I componenti di unlumen». |
| `cn.ts` | `cn(...)`: unisce i nomi di classe (`clsx`) e lascia vincere l’ultima classe di Tailwind in conflitto (`tailwind-merge`). Ogni `className` delle primitive passa da qui. |
| `focus-return.ts` | `useReturnFocus(ref)`: l’hook con cui `Modal` e `SidePanel` rimettono il focus a chi lo aveva quando compaiono. Vedi «Modal» e «SidePanel». |
| `README.md` | Questa guida. |

Le regole di una sola pagina, come `.calendar-event`, restano in `desktop/src/styles.css` o nel `.css` accanto al componente, annidate sotto la sua classe radice: non entrano nel modulo. I file del modulo non importano da `desktop/src/lib/`, da `desktop/src/components/` né da `@tauri-apps`, e i componenti di unlumen importano solo dal modulo (ADR-0011, INV-001): il modulo si può esportare e provare da solo. Le dipendenze ammesse sono quelle di ADR-0011 v3, a versione esatta, senza `^`: `react` e `react-dom` 19.3.1, `@ark-ui/react` 5.39.3, `motion` 14.7.1, `@internationalized/date` 3.12.4, `clsx` 2.1.1, `tailwind-merge` 3.7.0, `tailwindcss` e `@tailwindcss/vite` 4.3.3, `tw-animate-css` 1.4.0, e per la build `@vitejs/plugin-react` 6.1.2. Nessun altro kit, e nessuna dipendenza per il carattere: IBM Plex Sans sta in `desktop/public/fonts/ibm-plex-sans/`.

## Come costruire con questo sistema

Nell’app si usano le primitive React. Ognuna produce HTML con le classi di `system.css` e le parti `[data-scope][data-part]` di Ark; sotto ogni primitiva c’è quell’HTML, che è anche ciò che i test cercano (ruoli, nomi, classi) e ciò che si scrive a mano fuori dall’app.

- Non scrivere a mano le classi `primary`, `secondary`, `icon-button`, `text-button`, `popover`, `side-panel-layer` né l’elemento `<dialog>` nei componenti dell’app: sono dentro le primitive (ADR-0011, INV-002).
- Un menu di comandi è un `Menu`, non un `Popover`: un `Popover` i cui figli sono tutti pulsanti fa fallire `desktop/tests/ds-usage.mjs`. Il `Popover` resta per ciò che non è un menu: un filtro, un selettore, un piccolo modulo.
- Non scrivere `<select>`, `<input type="checkbox">`, `role="tablist"` a mano fuori dal modulo: sono `Select`, `Checkbox` o `Switch`, `Tabs` (ADR-0011, INV-003). L’unica eccezione è la casella di selezione di ogni riga della posta (`components/MailList.tsx`), che resta markup semplice (le righe non usano Ark, per i tempi). I campi `type="time"` restano nativi.
- Non importare `@ark-ui/*` nei componenti dell’app (INV-007): se manca una prop, la si aggiunge alla primitiva.
- Le primitive sono controllate: lo stato sta nel chiamante, che passa `value` (o `checked`, `open`) e lo aggiorna in `onChange` (o `onOpenChange`). Le classi di pagina si passano con `className`, e `cn` le unisce a quelle della primitiva; il nodo si chiede con `ref`.
- Un’icona da sola non basta come nome: ogni controllo a sola icona ha `aria-label`. `tip` si mette solo ai pulsanti a sola icona, e `shortcut` solo con una scorciatoia che esiste davvero.
- Le classi di Tailwind che i componenti di unlumen scrivono sui propri nodi (`relative inline-flex rounded-full…`) non sono un’API: regole di pagina e test scelgono sulle classi di questa guida, su ruoli e nomi e su `data-scope`/`data-part`.
- Prendi ogni colore, ombra, raggio, livello e dimensione del testo da un token (ADR-0009), nel CSS con `var(--…)` e nelle utility con i nomi di `tailwind.css`. Nell’app un valore letterale nel CSS o nel TSX fa fallire `desktop/tests/style-tokens.mjs`.
- Imposta il tema con `data-scheme="light"` o `data-scheme="dark"` sull’elemento `html`. Senza attributo vale il chiaro.

```tsx
import Button from '../ds/primitives/Button';

<div className="button-row">
  <Button variant="primary" type="button" onClick={save}>Salva modifiche</Button>
  <Button variant="secondary" type="button" onClick={cancel}>Annulla</Button>
</div>
```

L’HTML che produce, e che si scrive fuori dall’app:

```html
<div class="button-row">
  <button class="primary" type="button">Salva modifiche</button>
  <button class="secondary" type="button">Annulla</button>
</div>
```

## Le primitive

Sono diciassette componenti più l’icona. La loro forma è fissata: cambiare un controllo vuol dire cambiare un solo file, e cambiano tutti i punti dell’app che lo usano (ADR-0011). Ogni file ha un’esportazione predefinita, salvo `NavList.tsx` (`NavList` e `NavItem` per nome); `Toaster.tsx` esporta anche `notify` e il tipo `NoticeAction`, `Icon.tsx` anche `iconPaths`, `Menu.tsx` anche il tipo `MenuItem`. In tabella le props (quelle senza «?» sono obbligatorie; il resto degli attributi del nodo passa dove indicato); sotto, una sezione per primitiva con un esempio in TSX e l’HTML che produce.

| Primitiva | Basata su | Props | Lo stile sta in |
|---|---|---|---|
| `Button` | `<button>` (e `Tooltip` con `tip`) | `variant` (`primary`, `secondary`, `icon`, `text`); `className?`; `ref?`; `tip?`; `shortcut?`; gli attributi di `<button>` | `styles/system.css` |
| `Field` | `<label>` | `children` (etichetta e controllo); `className?`; gli attributi di `<label>` | `styles/system.css` |
| `Modal` | Ark `Dialog`, modale | `size?` (`sm`, `md`, `lg`; predefinito `md`); `onClose?`; `aria-label?` o `aria-labelledby`; `className?`; `children`; gli attributi di `<div>` | `styles/ark/dialog.css` |
| `SidePanel` | Ark `Dialog`, non modale | `onScrim` (obbligatoria); `narrow?`; `onClose?`; `onOpen?`; `ref?`; `aria-label?`; `className?`; `children`; gli attributi di `<div>` | `styles/ark/dialog.css` |
| `Popover` | Ark `Popover` | `trigger` (un elemento); `label`; `tip?`; `open?`; `onOpenChange?`; `className?`; `children` | `styles/ark/popover.css` |
| `Menu` | Ark `Menu` | `trigger` (un elemento); `label`; `items` (`{ value, label, onSelect, destructive?, disabled?, shortcut?, icon?, group?, checked?, title? }[]`); `tip?`; `open?`; `onOpenChange?` | `styles/ark/menu.css` |
| `Tooltip` | Ark `Tooltip` | `text`; `shortcut?`; `children` (l’unico elemento che fa da attivatore) | `styles/ark.css` |
| `Select` | Ark `Select` | `value` (`''` = niente); `onChange?`; `items` (`{ value, label, disabled? }[]`); `label?` o `aria-label`; `placeholder?`; `aria-required?`; `aria-invalid?`; `disabled?`; `className?` | `styles/ark/select.css` |
| `Checkbox` | Ark `Checkbox` | `checked`; `onChange?`; `indeterminate?`; `disabled?`; `aria-label?`; `children` (etichetta) | `styles/ark/checkbox.css` |
| `Switch` | `Switch` di Sigrid (`src/ui/Switch.tsx`) | `checked`; `onChange?`; `disabled?`; `aria-label?`; `children` (etichetta) | — |
| `Tabs` | Ark `Tabs` e Motion | `value`; `onChange?`; `items` (`{ value, label, count?, className?, props? }[]`); `aria-label`; `variant?` (`underline`, `segment`, `plain`); `activation?` (`automatic`, `manual`); `ids?`; `onPick?`; `tab?`; `after?` | `styles/ark/tabs.css` |
| `Slider` | Ark `Slider` | `value`; `onChange?`; `min`; `max`; `step?` (1); `aria-label` | `styles/ark/slider.css` |
| `DateField` | Ark `DatePicker` | `value` (`'YYYY-MM-DD'` o `''`); `onChange?`; `aria-label`; `min?`; `max?`; `locale?` (`en-US`); `disabled?`; `translations?` | `styles/ark/date-picker.css` |
| `RecipientsInput` | Ark `TagsInput` e `Combobox` | `value?` (`string[]`); `onChange?`; `suggestions`; `aria-label`; `onQuery?`; `invalid?`; `placeholder?`; `disabled?`; `inputRef?`; `pending?`; `onPendingChange?`; `translations?`; `listTranslations?`; `suggestionsLabel?`; `className?` | `styles/ark/recipients.css` |
| `Toaster` | Ark `Toast` | `label`; `closeLabel`; l’esportazione `notify(message, error?, actions?)` mostra un avviso, con al massimo due azioni (`NoticeAction = { label, run }`) | `styles/ark/toast.css` |
| `Icon` | SVG | `name`; `size?` (20); `stroke?` (1,7) | `primitives/Icon.tsx` |
| `Count` | Animate Digits di unlumen | `value` (`number` o `string`); `className?` | `styles/nav.css` (`.count-digits`) |
| `NavList` / `NavItem` | Sidebar 001 di unlumen, Motion | `NavList`: `group?` (`'nav'`); `className?`; gli attributi di `<nav>`. `NavItem`: `active?`; `hint?`; `className?`; gli attributi di `<button>` | `styles/nav.css` (`.nav-hover`, `.nav-active-bar`) |

### Button

Quattro varianti con `variant`: `primary`, `secondary`, `icon`, `text`. Rende un `<button>` con la classe `primary`, `secondary`, `icon-button` o `text-button`; `className` aggiunge classi di pagina, `ref` consegna il nodo, il resto (`onClick`, `disabled`, `aria-label`, `type`) passa al `<button>`. Senza `type`, dentro un `<form>` il pulsante invia: scrivi `type="button"`.

- **primary**: l’azione principale di una zona, al massimo una per gruppo. Riempimento `accent` (l’indaco), testo `on-accent`, altezza `control-height` (32px), raggio `radius-md` (6px), padding 0 12px, peso 600, nessuna ombra (`shadow-sm` vale `none`). In un gruppo il solo pulsante indaco.
- **secondary**: le alternative. Bordo `border`, riempimento `surface`, nessuna ombra; stessa altezza, raggio, padding e peso di primary.
- **icon**: quadrato di `control-height` (32×32px) con la sola icona in `muted`, raggio `radius-md`, per azioni ripetute e note dal contesto. Richiede sempre `aria-label`.
- **text**: azioni minori in linea, in `muted` e `text-xs`.

Stati: hover (`accent-hover` per primary, riempimento `hover` per secondary e icon; l’icona passa anche da `muted` a `text`; text non cambia), focus da tastiera (contorno di 2px in `accent` staccato di 3px; non toglierlo), disattivato (opacità 0,5, cursore normale). Alla pressione i pulsanti non cambiano scala.

```tsx
import Button from '../ds/primitives/Button';
import Icon from '../ds/primitives/Icon';

<Button variant="primary" type="button" onClick={reply}><Icon name="reply" size={16} />Rispondi</Button>
<Button variant="icon" type="button" aria-label="Archivia" onClick={archive}><Icon name="archive" size={18} /></Button>
```

```html
<button class="primary" type="button"><svg …/>Rispondi</button>
<button class="icon-button" type="button" aria-label="Archivia"><svg …/></button>
```

Con `tip` (e un `shortcut` facoltativo) il pulsante è l’attivatore di un `Tooltip`: il suggerimento nomina l’azione e la sua scorciatoia, ma non è il nome. Un pulsante a sola icona ha comunque il suo `aria-label`, e con `tip` non si scrive nessun `title`. Senza `tip` il pulsante è il `<button>` di sempre, senza involucro e senza codice del suggerimento. Con `tip` Ark unisce i suoi attributi e gestori a quelli del pulsante (`onClick` e gli altri si sommano). Un `Button` con `tip` non può fare da `trigger` di un `Popover` né da figlio di un altro `Tooltip`: l’elemento esterno sarebbe il `Tooltip`, che non passa attributi a nessun nodo.

```tsx
<Button variant="icon" type="button" tip="Archivia" shortcut="E" aria-label="Archivia" onClick={archive}><Icon name="archive" size={18} /></Button>
```

### Field

Un controllo di modulo con la sua etichetta: il testo dell’etichetta e il controllo sono i figli. Rende `<label class="field">`, quindi il controllo annidato è già associato all’etichetta. Il controllo (`input`, `textarea`) prende bordo `border-strong`, raggio `radius-md` (6px), padding 7px 9px e sfondo `surface` dalla regola base di `system.css`, senza ombra; al focus da tastiera prende il contorno di 2px in `accent` staccato di 3px, come ogni controllo. I componenti cambiano solo la larghezza, non ridefiniscono bordo, raggio, padding, sfondo o colore (ADR-0009, INV-002). Il testo di aiuto è un `p.field-help` accanto al campo.

```tsx
import Field from '../ds/primitives/Field';

const [name, setName] = useState('');

<Field><span>Nome</span><input value={name} onChange={(event) => setName(event.currentTarget.value)} /></Field>
<p className="field-help">Compare nell’elenco dei contatti.</p>
```

```html
<label class="field"><span>Nome</span><input /></label>
<p class="field-help">Compare nell’elenco dei contatti.</p>
```

### Modal

Il dialog centrato di ADR-0008: «devo decidere prima di continuare». È un `Dialog` di Ark UI modale, aperto appena compare e disegnato sopra tutto, nel `Portal` di Ark al livello `z-modal`: Ark gestisce velo, trappola del focus e Esc; solo Esc arriva come `onClose` (un clic sul velo non chiude: un modulo a metà non va perso). Chi lo usa lo chiude smontandolo; il `Modal` ricorda l’elemento che aveva il focus quando è comparso e glielo restituisce (`useReturnFocus`, in `focus-return.ts`, che usa anche il `SidePanel`) se il focus cadrebbe su `<body>`; se quell’elemento non c’è più perché era la voce di un menu o di un popover che si è chiuso, lo dà al pulsante di quel popup. Se il chiamante non smonta il `Modal`, resta aperto. Un solo velo (`scrim`), bordo `border`, raggio `radius-xl` (12px), ombra `shadow`, padding 22px; entra in `motion-panel` sfumando, salendo di 8px e crescendo dal 98%, mentre il velo sfuma. `size` sceglie la larghezza: senza, `modal-md` (560px); `sm` è `modal-sm` (420px), `lg` è `modal-lg` (960px). Il titolo è un `h2` (`text-xl` dentro il dialog), le azioni stanno in un `div.modal-actions`. `className` e gli altri attributi vanno sul contenuto. Monta il `Modal` solo quando serve, con `{asking && …}`.

L’ultima riga di azioni del dialog (`.modal-actions`, o una delle righe locali elencate in `styles/ark/dialog.css`) resta attaccata al fondo quando il contenuto è più alto della finestra, per esempio con il testo ingrandito: il resto scorre sotto di lei e una linea sottile la separa solo finché c’è altro contenuto sotto.

La finestra di composizione non è un `Modal`: resta una finestra agganciata in basso, con lo stesso raggio e la stessa ombra (ADR-0008, ADR-0011 v3, decisione 6).

```tsx
import Button from '../ds/primitives/Button';
import Modal from '../ds/primitives/Modal';

const [asking, setAsking] = useState(false);

{asking && (
  <Modal size="sm" aria-labelledby="delete-title" onClose={() => setAsking(false)}>
    <h2 id="delete-title">Eliminare il contatto?</h2>
    <p>Maria Rossi sparirà da tutte le rubriche.</p>
    <div className="modal-actions">
      <Button variant="secondary" type="button" onClick={() => setAsking(false)}>Annulla</Button>
      <Button variant="primary" type="button" onClick={remove}>Elimina</Button>
    </div>
  </Modal>
)}
```

```html
<!-- nel Portal, in <body> -->
<div data-scope="dialog" data-part="backdrop" data-variant="modal" data-state="open"></div>
<div data-scope="dialog" data-part="positioner" data-variant="modal">
  <div data-scope="dialog" data-part="content" data-variant="modal" data-size="sm" role="dialog" aria-modal="true" aria-labelledby="delete-title">
    <h2 id="delete-title">Eliminare il contatto?</h2>
    <p>Maria Rossi sparirà da tutte le rubriche.</p>
    <div class="modal-actions">…</div>
  </div>
</div>
```

### SidePanel

Il pannello laterale di ADR-0008: «guardo o modifico una cosa senza perdere il contesto». Entra da destra (24px, sfumando) in `motion-panel` con `ease-out` sopra `scrim-soft` e non è modale. Larghezza `panel-wide` (760px, gli allegati) o, con `narrow`, `panel-narrow` (420px, l’evento del calendario); sotto 1180px di finestra il pannello largo occupa tutta la finestra. Bordo sinistro `border` e ombra `shadow`. `onScrim` è obbligatorio: lo chiama il clic sul velo, che è solo una scorciatoia per il puntatore; le vie accessibili sono il pulsante di chiusura e Esc. È un `Dialog` di Ark UI non modale: niente trappola del focus, il resto della pagina resta raggiungibile con Tab. Esc chiama `onClose` (o `onScrim` se manca): chi lo usa non ascolta più il tasto. Chi usa il pannello lo smonta per chiuderlo, e smontato Ark non può restituire il focus: il `SidePanel` ricorda l’elemento che lo aveva quando è comparso (`useReturnFocus`, lo stesso del `Modal`) e glielo rimette alla chiusura, se è ancora nella pagina; se il pannello era stato aperto da una voce di un menu o di un popover che nel frattempo si è chiuso, il focus va al pulsante di quel popup. Lo fa solo se il focus è ancora dentro il pannello o su `<body>`: un focus che l’utente ha spostato sulla pagina dietro (il pannello non è modale) resta dov’è. `onOpen` parte quando è visibile, per mettere il focus (senza, lo riceve il primo controllo). `ref` consegna il nodo del pannello. Dentro: `.side-panel-bar` con `.panel-title` e un pulsante `panel-close`, poi `.side-panel-body`, che scorre. Il pannello non va in un portale: riempie il suo antenato posizionato, che deve essere `position: relative`.

```tsx
import { useRef } from 'react';
import Button from '../ds/primitives/Button';
import Icon from '../ds/primitives/Icon';
import SidePanel from '../ds/primitives/SidePanel';

const panel = useRef<HTMLDivElement>(null);
const closeButton = useRef<HTMLButtonElement>(null);

<SidePanel narrow aria-label="Design review" ref={panel} onScrim={close} onClose={close} onOpen={() => closeButton.current?.focus()}>
  <div className="side-panel-bar">
    <strong className="panel-title">Design review</strong>
    <Button variant="icon" type="button" className="panel-close" aria-label="Chiudi" title="Chiudi (Esc)" ref={closeButton} onClick={close}>
      <Icon name="close" size={16} />
    </Button>
  </div>
  <div className="side-panel-body">…</div>
</SidePanel>
```

```html
<div data-scope="dialog" data-part="positioner" data-variant="side">
  <div data-scope="dialog" data-part="backdrop" data-variant="side"></div>
  <div data-scope="dialog" data-part="content" data-variant="side" data-narrow role="dialog" aria-modal="false" aria-label="Design review">
    <div class="side-panel-bar">…</div>
    <div class="side-panel-body">…</div>
  </div>
</div>
```

### Popover

Il filtro, il selettore o il piccolo modulo ancorato al suo pulsante, il terzo contenitore di ADR-0008, su `Popover` di Ark UI: bordo `border`, raggio `radius-lg` (8px), ombra `shadow`, padding 5px, almeno 180px di larghezza, livello `z-popover`; entra sfumando e scendendo di 4px. Un menu di comandi non è un popover: è un `Menu` (sotto), e `ds-usage.mjs` rifiuta un `Popover` i cui figli sono tutti pulsanti. Props: `trigger` (l’unico elemento pulsante: Ark gli unisce i suoi attributi e il suo `ref`, quindi dev’essere un `<button>` o un `Button` senza `tip`), `label` (il nome accessibile della superficie, che è un `role="dialog"`), `tip` (il suggerimento di un pulsante a sola icona: il `Tooltip` avvolge l’attivatore di Ark, così gli attributi di entrambi arrivano al pulsante), `open` e `onOpenChange` per controllarlo, `className`. Senza `open` il popover tiene il proprio stato. Si apre dal pulsante e il focus va al primo controllo; Esc, un clic fuori o il focus che esce lo chiudono e il focus torna al pulsante; un’azione che ha finito il suo lavoro chiama `onOpenChange(false)`, cioè mette `open` a `false`. La superficie esiste solo da aperta, sotto il pulsante a destra, nel `Portal` di Ark. Un `Modal` aperto da dentro sta sopra: il primo Esc chiude il `Modal`, il secondo il popover. I `<button>` dentro la superficie prendono l’aspetto delle voci di un menu (padding 8px 9px, raggio `radius-sm` (4px), `hover` al passaggio), che era il primo uso del popover: un modulo con pulsanti veri li ridisegna, come «Registra ora» in `components/meetings/RecordNow.css`; `.popover-divider` separa due gruppi. Per contenuti lunghi non usare un popover: apri un pannello o un dialog.

```tsx
import { useState } from 'react';
import Button from '../ds/primitives/Button';
import Field from '../ds/primitives/Field';
import Popover from '../ds/primitives/Popover';

const [open, setOpen] = useState(false);
const [name, setName] = useState('');

<Popover
  open={open}
  onOpenChange={setOpen}
  label="Registra ora"
  className="record-popover"
  trigger={<Button variant="primary">Registra ora</Button>}
>
  <form className="record-form" onSubmit={(event) => { event.preventDefault(); start(name); setOpen(false); }}>
    <Field><span>Nome (facoltativo)</span><input value={name} onChange={(event) => setName(event.currentTarget.value)} /></Field>
    <div className="record-actions">
      <Button variant="primary" type="submit">Registra</Button>
    </div>
  </form>
</Popover>
```

```html
<button class="primary" type="button" data-scope="popover" data-part="trigger" aria-haspopup="dialog" aria-expanded="true" data-state="open">Registra ora</button>
<!-- aperto, nel Portal in <body> -->
<div data-scope="popover" data-part="positioner">
  <div data-scope="popover" data-part="content" class="record-popover" role="dialog" aria-label="Registra ora" data-state="open">
    <form class="record-form">…</form>
  </div>
</div>
```

### Menu

Il menu di comandi ancorato al suo pulsante («Altre azioni» del lettore, della rubrica e di una riunione), su `Menu` di Ark UI: per lo screen reader e per UI Automation è un menu (`role="menu"`, voci `menuitem`), non un dialog. Ha la superficie e le voci del `Popover` che i menu usavano, con le stesse misure e gli stessi token, quindi a vederlo non cambia nulla: bordo `border`, raggio `radius-lg` (8px), ombra `shadow`, padding 5px, almeno 180px di larghezza, livello `z-popover`, nel `Portal` di Ark sotto il pulsante a destra, solo da aperto; entra sfumando e scendendo di 4px (`motion-panel`, spento con `prefers-reduced-motion`). Le voci hanno padding 8px 9px, raggio `radius-sm` (4px) e l’icona di 15px a 8px dal testo; in un menu dove alcune voci hanno l’icona, quelle senza lasciano vuoto il suo posto, così le parole cominciano tutte sullo stesso bordo.

Props: `trigger` (l’unico elemento pulsante, come nel `Popover`: Ark gli unisce attributi e `ref`), `label` (il nome accessibile del menu: Ark lo chiamerebbe come il suo pulsante, la primitiva gli dà `label`), `items`, `tip` (il suggerimento di un pulsante a sola icona, come nel `Popover`), `onOpenChange`, che dice quando si apre e quando si chiude, e `open`: un cambio di `open` lo apre o lo chiude, come fa il lettore quando cambia il messaggio. Lo stato resta di Ark: un menu comandato da fuori renderebbe il focus al pulsante a ogni chiusura, anche dopo un clic su un altro controllo. Ogni voce di `items` è `{ value, label, onSelect, destructive?, disabled?, shortcut?, icon?, group?, checked?, title? }` (il tipo `MenuItem`):

- `value` la distingue; `label` è il suo nome, e ciò che cerca una lettera digitata; `onSelect` è il comando.
- `icon` è il nome di un’`Icon`, disegnata a 15px prima del testo.
- `group`: le voci dello stesso gruppo stanno insieme, e fra due gruppi va un separatore (`<hr role="separator">`, la linea di `.popover-divider`: 1px in `border`, 4px sopra e sotto).
- `destructive`: Elimina e simili vanno in fondo, dopo un separatore loro, dovunque siano nell’elenco.
- `disabled`: la voce resta visibile a metà opacità con `aria-disabled`, le frecce e le lettere la saltano e un clic non fa nulla; `title` dice perché, come «Messaggio inviato: si gestisce da Inviati».
- `checked` fa della voce un interruttore (`menuitemcheckbox` con `aria-checked`): accesa mostra il segno di spunta al posto dell’icona, come «Metti da parte».
- `shortcut` è una scorciatoia che l’app ha davvero: si vede in fondo alla voce, in un `<kbd>`, ed è annunciata come `aria-keyshortcuts`, non come parte del nome.

Tastiera, come la vuole Ark: Invio, Spazio o Giù sul pulsante aprono il menu sulla prima voce, Su sull’ultima (sul pulsante le frecce sono del menu: non passano al messaggio seguente, vedi «Tastiera e scorciatoie»); Giù e Su spostano fra le voci, Home e Fine vanno alla prima e all’ultima; una lettera porta alla voce che comincia così, la stessa lettera di nuovo alla seguente; Invio o Spazio scelgono; Tab non esce dal menu. Esc, la scelta di una voce e un clic fuori lo chiudono; con Esc e con la scelta il focus torna al pulsante, con un clic fuori resta dove l’ha messo il clic (su un altro controllo) o torna al pulsante (sul fondo della pagina). La primitiva rimette il focus sul pulsante già prima di eseguire il comando: un `Modal` o un `SidePanel` aperto dalla voce ricorda il pulsante e glielo rende quando si chiude. Mentre il menu è aperto il focus è sul menu, che indica la voce corrente con `aria-activedescendant`; Ark la segna con `data-highlighted`. Le lettere e le frecce restano al menu: le scorciatoie dell’app (`E`, `R`, `N`) non partono mentre il menu ha il focus (`shortcutCoveredByLayer` copre `role="menu"`).

Le due indicazioni sono quelle che avevano i pulsanti del popover: sotto il puntatore la voce prende il fondo `hover`; con la tastiera la voce corrente prende l’anello del focus, 2px in `accent` staccato di 3px, come ogni controllo. La primitiva scrive `data-keyboard` sul menu finché a guidare sono i tasti, perché `:focus-visible` qui non basta: Ark lascia il focus dov’era quando si preme il pulsante, e un menu aperto col mouse dopo un tasto premuto altrove risulterebbe aperto da tastiera. Con i temi Contrasto (`forced-colors.css`) il menu ha un bordo, la voce corrente un contorno tratteggiato in `Highlight`, il separatore è in `CanvasText` e una voce disattivata in `GrayText`.

```tsx
import Button from '../ds/primitives/Button';
import Icon from '../ds/primitives/Icon';
import Menu from '../ds/primitives/Menu';

<Menu
  label="Azioni sulla riunione"
  tip="Altre azioni"
  trigger={
    <Button variant="icon" aria-label="Altre azioni">
      <Icon name="more" size={18} />
    </Button>
  }
  items={[
    { value: 'rename', label: 'Rinomina', icon: 'edit', onSelect: () => ask('rename') },
    { value: 'delete', label: 'Elimina', icon: 'trash', destructive: true, onSelect: () => ask('delete') },
  ]}
/>
```

```html
<button class="icon-button" type="button" aria-label="Altre azioni" aria-haspopup="menu" aria-expanded="true" data-placement="bottom-end">…</button>
<!-- aperto, nel Portal in <body>: guidano i tasti, ed Elimina è la voce corrente -->
<div data-scope="menu" data-part="positioner">
  <div data-scope="menu" data-part="content" role="menu" tabindex="0" aria-label="Azioni sulla riunione" aria-labelledby="(il suo stesso id)" aria-activedescendant="…/delete" data-state="open" data-keyboard>
    <div data-scope="menu" data-part="item" role="menuitem" id="…/rename" data-value="rename" data-valuetext="Rinomina"><svg …/><span>Rinomina</span></div>
    <hr data-scope="menu" data-part="separator" role="separator" aria-orientation="horizontal">
    <div data-scope="menu" data-part="item" role="menuitem" id="…/delete" data-value="delete" data-valuetext="Elimina" data-highlighted><svg …/><span>Elimina</span></div>
  </div>
</div>
```

### Tooltip

Il suggerimento di un controllo senza spazio per le parole: nomina l’azione e, se c’è, la sua scorciatoia in un `<kbd>`. Props: `text`, `shortcut` facoltativa e `children`, l’unico elemento che fa da attivatore: Ark gli unisce attributi, gestori e `ref` (`asChild`), quindi dev’essere un elemento DOM o un componente che li passa al suo nodo, come `Button` senza `tip`. Di norma non si usa a mano: lo scrive `Button` quando ha `tip`, e `NavItem` quando ha `hint`; a mano serve per un `<button>` di pagina, come le righe della barra laterale compressa in `components/Sidebar.tsx`. È `Tooltip` di Ark UI (`@ark-ui/react/tooltip`) nel `Portal` di Ark, sotto il controllo a 8px. Si apre subito col focus da tastiera e sul passaggio del mouse dopo 400 ms; si chiude con Esc, quando il focus se ne va e quando il puntatore esce (dopo 100 ms); un tocco non lo apre. Esiste nel DOM solo mentre è aperto. L’aspetto è in `styles/ark.css`: un chip scuro sopra ogni fondo, nei colori della barra laterale (fondo `sidebar`, testo `sidebar-text-active`, bordo `sidebar-active`), `text-2xs`, raggio `radius-sm` (4px), padding 4px 8px, nessuna ombra, livello `z-toast`; il `<kbd>` è in `sidebar-muted` con bordo `sidebar-active`. Entra sfumando in `motion-panel`; l’animazione vale solo con `data-state="open"`, così un suggerimento che si chiude sparisce subito e non resta sullo schermo accanto al nuovo. Mostra solo le scorciatoie che l’app ha già (`N`, `R`, `E`, `Ctrl+K`, `Ctrl+Z`, frecce).

```tsx
import Tooltip from '../ds/primitives/Tooltip';

<Tooltip text="Archivia" shortcut="E">
  <button type="button" aria-label="Archivia" onClick={archive}><Icon name="archive" size={18} /></button>
</Tooltip>
```

```html
<button type="button" aria-label="Archivia" data-scope="tooltip" data-part="trigger" aria-describedby="…">…</button>
<!-- aperto, nel Portal in <body> -->
<div data-scope="tooltip" data-part="positioner">
  <div data-scope="tooltip" data-part="content" role="tooltip"><span>Archivia</span><kbd>E</kbd></div>
</div>
```

### Select

Una scelta da un elenco breve, su `Select` di Ark: un pulsante (`role="combobox"`, alto `control-height`) che mostra l’etichetta della voce scelta, e un elenco nel `Portal` di Ark al livello `z-toast`, quindi sopra `Modal` e `SidePanel` e usabile dentro di loro (Ark conta un clic o un tasto nell’elenco come interno al dialog). `value` è la voce scelta e `onChange` riceve il nuovo valore; `''` vuol dire nessuna scelta e mostra `placeholder` in `muted`. `items` è `{ value, label, disabled? }[]`. `label` è l’etichetta visibile e anche il nome del pulsante; senza, passa `aria-label`. `aria-required` e `aria-invalid` vanno sul pulsante. Tastiera: Invio, Spazio o le frecce aprono; le frecce muovono, Invio sceglie, Esc chiude solo l’elenco e il focus resta sul pulsante; una lettera salta alla prima voce che comincia con quella. Il pulsante ha bordo `border-strong`, raggio `radius-md`, fondo `hover` al passaggio e bordo `accent` da aperto. L’elenco esiste nel DOM solo da aperto, largo quanto il pulsante, con raggio `radius-lg` e ombra `shadow`; entra sfumando e scendendo di 4px; la voce scelta è in `accent-text`, peso 500, con un segno di spunta. Sostituisce ogni `<select>`.

```tsx
import Select from '../ds/primitives/Select';

const [language, setLanguage] = useState('it');

<Select
  label="Lingua"
  value={language}
  onChange={setLanguage}
  items={[
    { value: 'it', label: 'Italiano' },
    { value: 'en', label: 'English' },
  ]}
/>
```

```html
<div data-scope="select" data-part="root">
  <label data-scope="select" data-part="label">Lingua</label>
  <div data-scope="select" data-part="control">
    <button data-scope="select" data-part="trigger" role="combobox" aria-expanded="false"><span data-part="value-text">Italiano</span><div data-part="indicator"><svg …/></div></button>
  </div>
</div>
<!-- aperto, nel Portal in <body> -->
<div data-scope="select" data-part="positioner">
  <div data-scope="select" data-part="content" role="listbox">
    <div data-scope="select" data-part="item" role="option" aria-selected="true" data-state="checked"><span data-part="item-text">Italiano</span><div data-part="item-indicator"><svg …/></div></div>
    <div data-scope="select" data-part="item" role="option" aria-selected="false"><span data-part="item-text">English</span>…</div>
  </div>
</div>
```

### Checkbox

Una casella con la sua etichetta: i figli sono l’etichetta e un clic su tutta la riga la inverte. `checked` è lo stato, `onChange` riceve il nuovo valore; `indeterminate` disegna il trattino di «alcuni, non tutti» (lo stato spuntato è allora falso e un clic spunta). Senza etichetta visibile passa `aria-label`, che diventa il nome della casella. Quadrato di 16px con raggio `radius-sm` (4px), bordo `muted` (`text` al passaggio del mouse), riempito di `accent` da spuntato, con il segno in `on-accent`; il focus da tastiera è il contorno in `accent` staccato di 2px. Sostituisce ogni `<input type="checkbox">` fuori dalle righe della posta.

```tsx
import Checkbox from '../ds/primitives/Checkbox';

const [notify, setNotify] = useState(true);

<Checkbox checked={notify} onChange={setNotify}>Avvisami delle nuove risposte</Checkbox>
```

```html
<label data-scope="checkbox" data-part="root" data-state="checked">
  <div data-scope="checkbox" data-part="control" data-state="checked">
    <div data-scope="checkbox" data-part="indicator"><svg …/></div>
    <div data-scope="checkbox" data-part="indicator" hidden><svg …/></div>
  </div>
  <span data-scope="checkbox" data-part="label">Avvisami delle nuove risposte</span>
  <input type="checkbox" checked style="…visually hidden…" />
</label>
```

### Switch

L’interruttore acceso/spento per le impostazioni che valgono subito: i figli sono l’etichetta, un clic su tutta la riga lo inverte. `checked` è lo stato, `onChange` riceve il valore. Il controllo è l’Apple Switch di unlumen (`unlumen/apple-switch.tsx`) nella misura `xs`, che ha la geometria dello switch di «Compatto» (binario di 32×18px, pallino di 14px), e nel tono `app`, che ha i suoi colori: binario `muted` da spento che si riempie di `accent` da acceso, pallino in `on-accent` (bianco nel chiaro, quasi nero nello scuro), un alone appena accennato in `accent-pro` da acceso. È un `<button role="switch">` con `aria-checked`; il pallino corre su una molla di Motion e si allarga quando lo si afferra, e si può anche trascinare. Non è una parte di Ark, ma porta l’anatomia dello switch di Ark che ha sostituito, su cui scelgono i test e l’esportazione: `data-scope="switch"` con `data-part="root"` sulla riga, `"control"` sul pulsante, `"thumb"` sul pallino e `"label"` sul testo, e `data-state` (`checked`/`unchecked`) su riga, pulsante e pallino, più `data-disabled` su riga e pulsante da disattivato. La riga è `label.switch-row` (o `span.switch-row` senza figli, e allora serve `aria-label`), il testo è `span.switch-label`, e `styles/ark/switch.css` disegna riga (spazio di 10px, `text-sm`), stato disattivato (opacità 0,5) e contorno del focus (2px in `accent`, staccato di 2px). Per una scelta che si conferma altrove (un modulo con «Salva») usa `Checkbox`.

```tsx
import Switch from '../ds/primitives/Switch';

const [sync, setSync] = useState(false);

<Switch checked={sync} onChange={setSync}>Sincronizza in background</Switch>
```

```html
<label class="switch-row" data-scope="switch" data-state="unchecked" data-part="root">
  <button id="…" type="button" role="switch" aria-checked="false" class="relative inline-flex … rounded-full …" style="width: 32px; height: 18px; touch-action: pan-y" data-scope="switch" data-state="unchecked" data-part="control">
    <span …></span><!-- alone -->
    <span …><span …></span><span data-part="fill" …></span></span><!-- binario e riempimento -->
    <span …></span><!-- la goccia che si vede quando lo si afferra -->
    <span data-scope="switch" data-part="thumb" data-state="unchecked" …></span>
  </button>
  <span class="switch-label" data-scope="switch" data-part="label">Sincronizza in background</span>
</label>
```

### Tabs

La fila di schede di un elenco: disegna solo la lista, il contenuto sotto resta a chi la usa. `value` è la scheda scelta e `onChange` riceve la nuova; `items` è `{ value, label, count?, className?, props? }[]` e `aria-label` è obbligatorio. La scheda scelta porta l’evidenza scorrevole di unlumen: uno `span[data-part="highlight"]` con un `layoutId` di Motion che scivola su una molla verso la nuova scelta, e sopra di lui il testo in `span[data-part="trigger-label"]`. `variant`: `underline` (la scheda dell’elenco: 34px, peso 500 e 600 per la scelta, l’evidenza è una barra di 2px in `accent` sotto la scelta, il conteggio in `muted`), `segment` (il chip delle impostazioni, alto `control-height`: l’evidenza è un riquadro `surface` con bordo `border` e raggio `radius-sm` su un fondo `label-bg`, senza ombra perché `shadow-sm` vale `none`), `plain` (nessuna evidenza e nessun aspetto suo, per chi stila le schede da sé, come le bozze del dock). Frecce, Home e Fine muovono e scelgono; con `activation="manual"` muovono soltanto e scelgono Invio o Spazio. Per schede con più di un’etichetta: `ids` dà a scheda e pannello un id per valore (`{ trigger, content }`), `props` di una voce va sulla sua scheda (`title`, `aria-*`), `tab` disegna l’interno della scheda, `after` ciò che le sta accanto (un pulsante di chiusura, che non può stare dentro un `<button>`: scheda e `after` stanno allora in un `div[data-part="item"]` con il `className` della voce), `onPick` sente ogni clic, anche sulla scheda già scelta. Senza `ids` nessun pannello è disegnato e la scheda non porta `aria-controls`. Sostituisce ogni `role="tablist"` scritto a mano.

```tsx
import Tabs from '../ds/primitives/Tabs';

const [view, setView] = useState('smart');

<Tabs
  aria-label="Filtro dell’elenco"
  value={view}
  onChange={setView}
  items={[
    { value: 'smart', label: 'Tutte' },
    { value: 'unread', label: 'Da leggere', count: 12 },
  ]}
/>
```

Con `tab`, `after` e `onPick`, come nelle schede delle bozze:

```tsx
<Tabs
  variant="plain"
  aria-label="Bozze aperte"
  value={current}
  onChange={setCurrent}
  onPick={restore}
  items={drafts.map((draft) => ({ value: draft.id, label: draft.subject, props: { title: draft.subject } }))}
  tab={(item) => <span className="draft-title">{item.label}</span>}
  after={(item) => (
    <Button variant="icon" type="button" aria-label={`Chiudi ${item.label}`} onClick={() => discard(item.value)}>
      <Icon name="close" size={14} />
    </Button>
  )}
/>
```

```html
<div data-scope="tabs" data-part="root" data-orientation="horizontal" data-variant="underline">
  <div data-scope="tabs" data-part="list" role="tablist" aria-label="Filtro dell’elenco">
    <button data-scope="tabs" data-part="trigger" role="tab" aria-selected="true" data-selected>
      <span aria-hidden="true" data-part="highlight"></span>
      <span data-part="trigger-label">Tutte</span>
    </button>
    <button data-scope="tabs" data-part="trigger" role="tab" aria-selected="false">
      <span data-part="trigger-label">Da leggere<span class="count">12</span></span>
    </button>
  </div>
</div>
```

### Slider

Un cursore a un solo valore, per esempio la posizione del player delle riunioni. `value` è il valore e `onChange` riceve ognuno; `min` e `max` sono obbligatori, `step` vale 1, `aria-label` è obbligatorio. Sinistra e Giù abbassano, Destra e Su alzano di `step`; PagSu e PagGiù di dieci passi; Home e Fine vanno a `min` e `max`; il valore non esce mai da lì. Traccia di 4px in `border-strong`, tratto in `accent`, pomello di 16px (fondo `surface`, bordo di 2px in `accent`) che porta il contorno del focus, staccato di 2px.

```tsx
import Slider from '../ds/primitives/Slider';

const [position, setPosition] = useState(0);

<Slider aria-label="Posizione nella registrazione" min={0} max={3600} step={5} value={position} onChange={setPosition} />
```

```html
<div data-scope="slider" data-part="root">
  <div data-scope="slider" data-part="control">
    <div data-scope="slider" data-part="track"><div data-scope="slider" data-part="range"></div></div>
    <div data-scope="slider" data-part="thumb" role="slider" aria-label="Posizione nella registrazione" aria-valuemin="0" aria-valuemax="3600" aria-valuenow="0" tabindex="0"><input hidden /></div>
  </div>
</div>
```

### DateField

Un campo data: una casella di testo che accetta una data scritta e un pulsante che apre un calendario. `value` è la stringa `'YYYY-MM-DD'`, o `''` per nessuna data, ed è l’unica cosa che attraversa il confine; `onChange` riceve la nuova. Il testo mostra la data come la scrive `locale` (giorno per primo per `it-IT`, mese per primo per `en-US`); scrivendo si accetta quella forma e quella ISO. Il testo scritto si legge con Invio e quando il focus esce: una data vera chiama `onChange` (riportata dentro `min`..`max` se fuori), il testo vuoto la svuota, il resto (`2026-13-40`) è rifiutato e la casella torna all’ultima data buona, con `aria-invalid` finché è sbagliata (bordo `danger`). `min` e `max` limitano i giorni; `disabled` blocca casella e pulsante. Il calendario non va in un portale: resta dentro il campo, quindi fa parte del `Modal` o del `SidePanel` che lo contiene (trappola del focus, Esc), e `fixed` evita che il corpo scorrevole di un dialog lo tagli. Ha le viste giorno, mese e anno, in un riquadro di 252px con raggio `radius-lg` e ombra `shadow`; oggi è in `accent-text`, il giorno scelto è pieno di `accent` (e col focus il suo contorno è in `on-accent`, perché uno in `accent` sparirebbe). `translations` (la forma di Ark: i nomi dei pulsanti del calendario e le lettere del segnaposto) li scrive nella lingua dell’app; senza, sono in inglese: l’app lo passa dalla sua lingua. Alto `control-height`. I campi dell’ora restano `<input type="time">` nativi di classe `time-field`.

```tsx
import DateField from '../ds/primitives/DateField';

const [day, setDay] = useState('2026-11-08');

<DateField aria-label="Data di invio" value={day} onChange={setDay} locale="it-IT" min="2026-11-08" translations={translations} />
```

```html
<div data-scope="date-picker" data-part="root">
  <div data-scope="date-picker" data-part="control">
    <input data-scope="date-picker" data-part="input" type="text" autocomplete="off" aria-label="Data di invio" value="08/11/2026" />
    <button data-scope="date-picker" data-part="trigger" aria-label="Apri il calendario"><svg …/></button>
  </div>
  <!-- aperto, dentro il campo -->
  <div data-scope="date-picker" data-part="positioner">
    <div data-scope="date-picker" data-part="content">
      <div data-scope="date-picker" data-part="view" data-view="day">…intestazione, giorni della settimana, settimane…</div>
    </div>
  </div>
</div>
```

### RecipientsInput

I destinatari di un messaggio, su `TagsInput` (i chip) e `Combobox` (i suggerimenti) con un solo campo di testo. `value` è un elenco di voci, ognuna un indirizzo («ann@x.it») o un nome con indirizzo («Ann Rossi <ann@x.it>»), e `onChange` riceve il nuovo elenco; ogni voce è un chip che si toglie, e lo stesso indirizzo non compare due volte. Scrivendo il campo chiede i suggerimenti a chi lo usa (`onQuery`, con il testo dopo l’ultimo separatore; `suggestions` è la risposta, `{ email, name? }[]`, mostrata in un elenco nel `Portal` di Ark a `z-toast`, senza gli indirizzi già scelti). Frecce muovono, Invio sceglie, Esc chiude solo l’elenco (con l’elenco vuoto Esc arriva alla finestra intorno, che si riduce a icona). Invio, virgola o punto e virgola trasformano il testo in chip, come lasciare il campo; incollare un elenco lo spezza a virgole e punti e virgola; Backspace sul campo vuoto toglie subito l’ultimo chip. Finché non è un chip, il testo scritto è `pending`: chi lo controlla con `pending` e `onPendingChange` può contarlo come parte del valore (per esempio prima di inviare). `invalid(email)` segna (`data-invalid`, in `danger`) i chip il cui indirizzo è rifiutato. Le frasi vengono da `translations` (chip: «Remove …» se mancano) e `listTranslations` (elenco), l’inglese di Ark se mancano; `suggestionsLabel` è il nome dell’elenco (`Suggestions` se manca); `inputRef` consegna il campo per dargli il focus. Il campo è `role="combobox"`: nei test lo si trova per ruolo e nome.

```tsx
import { useRef, useState } from 'react';
import RecipientsInput from '../ds/primitives/RecipientsInput';

const [to, setTo] = useState<string[]>([]);
const [typed, setTyped] = useState('');
const [suggestions, setSuggestions] = useState<{ email: string; name?: string }[]>([]);
const field = useRef<HTMLInputElement>(null);

<RecipientsInput
  aria-label="A"
  value={to}
  onChange={setTo}
  suggestions={suggestions}
  onQuery={(text) => ask(text).then(setSuggestions)}
  invalid={(email) => !email.includes('@')}
  inputRef={field}
  pending={typed}
  onPendingChange={setTyped}
  suggestionsLabel="Suggerimenti"
/>
```

```html
<div data-scope="combobox" data-part="root">
  <div data-scope="combobox" data-part="control">
    <div data-scope="tags-input" data-part="control">
      <div data-scope="tags-input" data-part="item"><div data-part="item-preview" title="Ann Rossi <ann@x.it>"><span data-part="item-text">Ann Rossi</span><button data-part="item-delete-trigger" aria-label="Remove Ann Rossi <ann@x.it>"><svg …/></button></div></div>
      <input data-part="input" role="combobox" aria-label="A" />
    </div>
  </div>
</div>
<!-- aperto, nel Portal in <body> -->
<div data-scope="combobox" data-part="positioner">
  <div data-scope="combobox" data-part="content" role="listbox" aria-label="Suggerimenti">
    <div data-scope="combobox" data-part="item" role="option"><span data-part="item-text">Maria Rossi</span><small>maria@x.it</small></div>
  </div>
</div>
```

### Toaster

Gli avvisi dell’app, su `Toast` di Ark: se ne vede uno alla volta, in basso al centro a `z-toast` (con una finestra di scrittura aperta si alza sopra la sua barra, con `--toast-bottom`). Si monta una volta sola, alla radice dell’app, con `label` (il nome della regione) e `closeLabel` (il nome del pulsante di chiusura): le parole le dà il chiamante, il modulo non legge le traduzioni dell’app. Si mostra un avviso con `notify(message, error, actions)` esportato dallo stesso file: un nuovo avviso sostituisce il vecchio; un esito è uno `status` con la spunta in `success` che si chiude da solo dopo 4,5 s, un errore è un `alert` con bordo `danger-border` che resta 12 s; restano aperti finché il puntatore o il focus sono sopra. Con `actions` (al massimo due, `{ label, run }`; le altre si ignorano) l’avviso porta altrettanti pulsanti `secondary` tra il messaggio e il pulsante di chiusura, raggiungibili con Tab: un clic esegue `run` e chiude l’avviso, e con azioni l’avviso resta 10 s (almeno), perché c’è qualcosa da decidere. Le azioni sono una comodità, mai l’unica strada: quello che fanno si deve poter fare anche altrove (il file salvato resta in Esplora risorse). Superficie `surface`, bordo `border`, raggio `radius-lg` (8px), ombra `shadow`, come un popover; il testo si può selezionare. Scorre al suo posto in `motion-panel`. La scorciatoia propria di Ark (Alt+T) è spenta. `UndoStack` non passa da qui: il suo conto alla rovescia batte ogni 50 ms e `tests/undo.mjs` conta quel battito, mentre `Toast` ha i suoi timer (ADR-0011).

```tsx
import Toaster, { notify } from '../ds/primitives/Toaster';

<Toaster label="Notifiche" closeLabel="Chiudi la notifica" />
<Button variant="primary" type="button" onClick={() => notify('Salvato')}>Salva</Button>
// un errore: notify('Impossibile salvare il contatto. Riprova tra poco.', true)
// con azioni: notify('Allegato salvato', false, [{ label: 'Apri', run: apri }, { label: 'Mostra nella cartella', run: mostra }])
```

```html
<div data-scope="toast" data-part="group" aria-label="Notifiche">
  <div data-scope="toast" data-part="root" data-type="success" class="toast" role="status">
    <svg>…</svg>
    <div data-scope="toast" data-part="description">Salvato</div>
    <!-- con azioni, prima del pulsante di chiusura: -->
    <button class="secondary toast-action" type="button">Apri</button>
    <button data-scope="toast" data-part="close-trigger" class="icon-button" aria-label="Chiudi la notifica">…</button>
  </div>
</div>
```

### Icon

SVG a contorno, griglia 24×24, tratto 1,7, estremità e angoli arrotondati, in `currentColor`: l’icona prende il colore del testo. `name` sceglie il tracciato; un nome sconosciuto ricade su `mail`. `size` è il lato in px (predefinito 20; i più usati sono 16 e 18, e 12, 14 e 15 nei controlli compatti), `stroke` lo spessore (la barra laterale usa 1,8). Il tracciato di ogni icona sta in un solo posto, la tabella `iconPaths` di `primitives/Icon.tsx`: per aggiungere un’icona si aggiunge lì una voce. L’icona è `aria-hidden`: il nome accessibile lo dà il pulsante che la contiene. Le icone dell’app vengono tutte da qui; `lucide-react` è ammesso solo perché lo importano i componenti di unlumen.

```tsx
import Icon from '../ds/primitives/Icon';

<Icon name="calendar" size={18} />
```

```html
<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="…" /></svg>
```

### Count

Un numero che cambia sul posto: i conteggi della barra laterale, i totali. È Animate Digits di unlumen (`unlumen/animate-digits.tsx`): ogni cifra che cambia scorre su una molla, sfocata e un poco rimpicciolita mentre entra, e le cifre che non cambiano restano ferme. `value` è il numero (o un testo), `className` le classi di pagina (`nav-count` nella barra laterale). Il testo dell'elemento sono le sole cifre correnti (quelle che escono sono `aria-hidden`), quindi testo e nome accessibile sono il numero semplice. Con `prefers-reduced-motion` la primitiva rende il solo numero, senza Motion. Il colore, la dimensione e le cifre tabellari vengono dalla classe di pagina; `.count-digits` (in `styles/nav.css`) mette le celle in fila. Non usarlo per un numero che non cambia sotto gli occhi: per quello basta il testo.

```tsx
import Count from '../ds/primitives/Count';

{!!workspace.counts.Inbox && <Count value={workspace.counts.Inbox} className="nav-count" />}
```

```html
<span class="count-digits nav-count">
  <span class="inline-flex items-center tabular-nums">…una cella per cifra, animata (1, 2)…</span>
</span>
<!-- con prefers-reduced-motion -->
<span class="nav-count">12</span>
```

### NavList e NavItem

Le righe di navigazione nel modo di Sidebar 001 di unlumen (la primitiva ne riprende i due effetti; il file di unlumen non è copiato): un’evidenza morbida segue il puntatore da una riga all’altra su una molla, e la riga corrente porta una barretta di 3×14px in `accent-on-sidebar` (l’indaco schiarito per la barra laterale scura) che scivola verso la nuova riga corrente (un `layoutId` di Motion condiviso). `NavList` rende un `<nav class="nav-list">` (con `aria-label`); `group` (predefinito `'nav'`) dice fra quali liste scivola la barra: liste con lo stesso `group` ne hanno una sola, come cartelle, etichette e strumenti nella barra laterale, mentre gli account hanno la loro. `NavItem` rende un `<button>` con `data-nav-item`; `active` gli dà la classe `active` e la barra, `hint` lo fa attivatore di un `Tooltip` (la barra laterale compressa, dove una riga è solo un’icona); il resto (`onClick`, `aria-current`, `aria-pressed`, `aria-label`, `title`) passa al `<button>`. La riga corrente di una navigazione dichiara `aria-current="page"`, una scelta come l’account `aria-pressed`. Con `prefers-reduced-motion` l’evidenza che segue il puntatore non compare. L’evidenza (`sidebar-hover`, raggio `radius-md`) e la barra stanno in `styles/nav.css` (`.nav-hover`, `.nav-active-bar`); l’aspetto delle righe della barra laterale resta in `desktop/src/styles.css` (`.sidebar nav button`: testo `sidebar-text`, la riga corrente una pillola `sidebar-active` in `sidebar-text-active`, peso 500, con l’icona in `accent-on-sidebar`). Un esempio vero è `components/Sidebar.tsx`.

```tsx
import Count from '../ds/primitives/Count';
import Icon from '../ds/primitives/Icon';
import { NavItem, NavList } from '../ds/primitives/NavList';

<NavList aria-label="Cartelle della posta" group="views">
  {primary.map(([id, icon]) => (
    <NavItem
      key={id}
      hint={collapsed ? folderLabel(id) : undefined}
      active={view === id}
      onClick={() => navigate(id)}
      aria-label={collapsed ? folderLabel(id) : undefined}
      aria-current={view === id ? 'page' : undefined}
    >
      <Icon name={icon} size={16} stroke={1.8} />
      <span className="sidebar-text">{folderLabel(id)}</span>
      {!!workspace.counts[id] && <Count value={workspace.counts[id]} className="nav-count" />}
    </NavItem>
  ))}
</NavList>
```

```html
<nav class="nav-list" aria-label="Cartelle della posta">
  <span aria-hidden="true" class="nav-hover" style="top: …; height: …"></span><!-- solo al passaggio del puntatore -->
  <button class="active" data-nav-item="" aria-current="page">
    <span aria-hidden="true" class="nav-active-bar"></span>
    <svg …/><span class="sidebar-text">Posta in arrivo</span><span class="count-digits nav-count">…</span>
  </button>
  <button data-nav-item=""><svg …/><span class="sidebar-text">…</span></button>
</nav>
```

## I componenti di unlumen

unlumen UI è un registro di componenti nello stile di shadcn/ui (React, Tailwind, Motion): il codice si copia nel progetto, non si installa. In `desktop/src/ds/unlumen/` ci sono solo componenti gratuiti, copiati e leggibili, che non cambiano da soli (ADR-0011 v3, decisione 3):

| File | Da unlumen | A cosa serve |
|---|---|---|
| `apple-switch.tsx` | Apple Switch | Il controllo di `Switch`. Modificato: la misura `xs` (la geometria dello switch di «Compatto»: binario di 32×18px, pallino di 14px) e il tono `app` (spento `muted`, acceso `accent`, pallino `on-accent`, alone `accent-pro`). |
| `animate-digits.tsx` | Animate Digits | Le cifre di `Count`. |

I componenti Pro di unlumen (Motion Tabs Menu, Sidebar 002, Animated Input) richiedono una licenza e non sono inclusi. Al loro posto `Tabs` riprende l’evidenza scorrevole delle primitive libere di unlumen (`tabs`, `highlight`) con un `layoutId` di Motion; `Switch` non ha l’Animated Input. Non copiare codice Pro nel repository.

Per aggiungere un componente:

1. Copialo dal registro (ui.unlumen.com, solo componenti gratuiti) in `unlumen/<nome>.tsx`, così com’è.
2. Togli la riga `"use client"`: non c’è Next.js.
3. Importa `cn` da `../cn` al posto del percorso di shadcn (`@/lib/utils`). Il file importa solo dal modulo e dalle dipendenze di ADR-0011 v3 (`react`, `motion`, `clsx`, `tailwind-merge`); una dipendenza nuova richiede una modifica dell’ADR.
4. Fai leggere i colori ai token (INV-008): i nomi di Tailwind di `styles/tailwind.css` (`bg-background`, `text-muted-foreground`, `bg-accent-pro`…) li leggono già; un colore scritto nel file diventa `var(--…)` dove decide l’aspetto. Un colore della tavolozza di Tailwind (`bg-white/10`, `bg-neutral-900`) non produce nulla, perché quella tavolozza è spenta: va sostituito con un nome della tabella. `style-tokens.mjs` lascia fuori `ds/unlumen/` (INV-004), quindi qui il controllo è tuo: i valori letterali che restano (le ombre e i riflessi del pallino di Apple Switch, i colori dei toni `neutral` e `accent`, che l’app non usa) sono quelli dell’originale. Le misure e i raggi seguono «Compatto», non l’originale: se il componente sostituisce un controllo che c’era, riprendine la geometria, come la misura `xs` di Apple Switch.
5. Annota la modifica in `unlumen/README.md`: da quale componente viene, che cosa hai cambiato rispetto all’originale, quale primitiva lo usa.
6. Se l’app lo usa, avvolgilo in una primitiva di `primitives/`, come `Switch` e `Count`, e aggiungi la sua sezione a questa guida.

## Tailwind

Tailwind CSS v4 entra con `@tailwindcss/vite`; `styles/tailwind.css` importa il tema e le utility, non il preflight: la base resta `system.css`, e il contenuto delle email ricevute conserva i valori del browser. `tw-animate-css` aggiunge le classi d’animazione che i componenti di unlumen usano. La variante `dark:` vale dentro `[data-scheme='dark']`.

I nomi di colore (`bg-paper`, `text-ink`, `bg-raised`, `bg-primary`…) sono quelli del tema di Sigrid (`vendor/sigrid/src/styles/theme.css`, importato da `tailwind.css`), con i valori di «Compatto» di `src/theme/compatto.css`; la tavolozza propria di Tailwind è spenta (`--color-*: initial`) prima che Sigrid nomini la sua. La vecchia mappa dei nomi di unlumen e shadcn sui token di Qamille non c’è più: le regole di pagina leggono i token di Qamille (alias dei token di Sigrid), le classi di Tailwind leggono quelli di Sigrid, e i due dicono lo stesso colore. Tailwind genera anche le classi che Sigrid scrive nei suoi sorgenti (`@source` su `vendor/sigrid/src`).

Il cursore resta la freccia: Sigrid ha ancora `cursor-not-allowed` e `cursor-progress` sui controlli disattivati (richiesta R16), e `system.css` tiene la freccia su di essi (regola 3, controllo 29 di `native-feel.mjs`).

## Le regole di `ark.css`

`styles/ark.css` e `styles/ark/*.css` dicono come appaiono le parti delle primitive. Ark dà il comportamento e scrive su ogni parte `data-scope`, `data-part` e `data-state`; le regole scelgono su quelli, quindi una parte non ha bisogno di una classe sua.

- **Selettore.** `[data-scope='select'][data-part='trigger']`, mai una classe generata da Tailwind e mai il nome di una classe interna di Ark. Lo stato è `[data-state]` o l’attributo booleano (`[data-disabled]`, `[data-highlighted]`, `[data-selected]`, `[data-invalid]`). Dove due primitive della stessa famiglia si distinguono, lo fa l’attributo `data-variant` o `data-size` che la primitiva scrive (`Modal`: `data-variant="modal"` e `data-size`; `SidePanel`: `data-variant="side"` e `data-narrow`; `Tabs`: `data-variant`). Le parti che il modulo aggiunge da sé seguono la stessa forma: `data-part="highlight"` e `data-part="trigger-label"` in `Tabs`, `data-part="item"` attorno a scheda e `after`. Fa eccezione `switch.css`, che sceglie su `.switch-row` e `button[role='switch']`, perché Apple Switch non è una parte di Ark (le sue parti portano comunque `data-scope="switch"` e `data-part`, per i test e l’esportazione).
- **Solo token.** Ogni colore, raggio, ombra, livello, dimensione del testo e durata viene da `theme.css`; vale `node tests/style-tokens.mjs` (ADR-0009). Altezza dei controlli: `--control-height`.
- **Un file per primitiva.** Una primitiva nuova porta il suo `styles/ark/<nome>.css` e la riga `@import` in testa a `ark.css`, scritta come stringa (`@import './ark/<nome>.css';`): gli `@import` vanno prima di ogni regola, altrimenti il CSS li ignora. Le regole del suggerimento (`tooltip`) stanno in `ark.css`.
- **Livelli.** Le parti che escono dal flusso stanno sulla scala `--z-popover` < `--z-panel` < `--z-modal` < `--z-toast`, nessun valore nuovo: `Popover` e `Menu` a `z-popover`, `SidePanel` a `z-panel`, `Modal` a `z-modal`, e a `z-toast` le liste di `Select` e dei suggerimenti dei destinatari, il suggerimento e gli avvisi, che devono vedersi sopra un dialog. `Modal`, `Popover`, `Menu`, `Tooltip`, la lista di `Select` e quella dei suggerimenti stanno nel `Portal` di Ark, in `<body>`: in React il contenuto del portale vive e muore con l’albero che lo contiene. `SidePanel` e il calendario di `DateField` restano dove sono.
- **Specificità bassa dove serve.** Le regole dei contenitori (`dialog.css`) sono in `:where()`: una classe di pagina sulla primitiva (la stampa, la scheda contatto) le ridefinisce.
- **Movimento.** Le entrate in CSS usano `motion-panel` ed `ease-out`, e ogni file le spegne sotto `prefers-reduced-motion`. Il suggerimento anima solo `[data-state='open']`: con l’animazione su ogni stato, Ark aspettava la fine dell’animazione prima di smontarlo e un suggerimento chiuso restava sullo schermo accanto al nuovo. Vedi «Stati e movimento».

## Tastiera e scorciatoie

- **Esc a strati.** Ark ascolta Esc su `document` in fase di cattura e solo lo strato più alto reagisce: la lista di un `Select`, i suggerimenti, il calendario di `DateField`, un `Popover`, un `Menu`, un `Modal` o un `SidePanel` prendono Esc prima di tutto il resto, ovunque sia il focus. La finestra di scrittura si riduce a icona con Esc solo se nessuno strato è aperto e il focus è al suo interno (il suo gestore controlla `defaultPrevented`). Un `Modal` aperto da una voce di un `Popover` sta sopra: il primo Esc chiude il `Modal`, il secondo il popover.
- **Guardia delle scorciatoie.** Le scorciatoie a tasto singolo del componente radice dell’app (`N`, `R`, `E`, le frecce, `Ctrl+Z`) passano da `shortcutIgnored` in `desktop/src/lib/shortcut-guard.ts`: ignorano i tasti premuti dentro `input`, `textarea`, `select`, un’area `contenteditable` e dentro le parti di Ark che leggono i tasti da sé (`[data-scope]` di `select`, `combobox`, `tags-input`, `date-picker`, `slider`, e `[role="slider"]`). Così scrivere una lettera per cercare in un elenco non archivia il messaggio. Una primitiva nuova che legge i tasti aggiunge qui il suo `data-scope` e un caso in `tests/shortcut-guard.mjs`. Un dialog, un popover o un menu che ha il focus (`role="menu"`, come `Menu`) tiene per sé tutti i tasti, e un `Modal` aperto anche senza il focus (`shortcutCoveredByLayer`). Le frecce passano da un messaggio all’altro solo dalla pagina, dall’elenco o dal lettore (`arrowsMoveMail`), e mai da un pulsante che apre un menu (`aria-haspopup="menu"`): lì Giù e Su aprono il menu.
- **Suggerimenti.** `tip`, `shortcut` e `hint` mostrano solo le scorciatoie che l’app ha già (`N`, `R`, `E`, `Ctrl+K`, `Ctrl+Z`, frecce): non se ne inventano.

## Colore

I colori di Sigrid sono calcolati dal suo motore dei temi a partire dai parametri di «Compatto» (`src/theme/compatto.ts`) e scritti in `src/theme/compatto.css` (`npm run theme:css`, file generato: non si modifica a mano). I nomi di Qamille per i token che Sigrid possiede (`ground`, `surface`, `text`, `muted`, `border`, `accent`, `selection`, `sidebar*`, `danger`, `success`…) sono alias di quelli di Sigrid in quel file; in `tokens/theme.css` restano solo i token che esistono solo in Qamille (priorità, etichette, avatar, scala dei titoli, raggi, livelli, movimento). La tabella prima/dopo dei valori che sono cambiati è in `tests/tokens-compatto.mjs`. Qui i ruoli. I componenti di unlumen non portano colori propri e leggono questi token (INV-008). I valori citati sotto sono quelli di «Compatto» prima del passaggio a Sigrid dove non diversamente detto.

- **Un solo accento**: l’indaco `accent` (`#4f46e5` nel chiaro, schiarito in `#9a9cf8` nello scuro), con `accent-hover` in hover e `on-accent` per il testo sopra (bianco nel chiaro, quasi nero nello scuro); `accent-text` per testo in accento su `surface` (`#4338ca`; `#aeb0fa` nello scuro), `selection` e `badge`. Serve solo a non letti, selezione, azione principale e conteggi: è il riempimento del pulsante primario, della casella spuntata, del giorno scelto, del tratto del cursore, della barra della scheda scelta, dell’interruttore acceso e del marchio, e il colore del focus. Non introdurre un secondo colore d’azione.
- **I nomi di unlumen**: `accent-pro` e `ring` sono i colori che leggono i componenti di unlumen (l’alone di Apple Switch, `bg-accent-pro` e `ring-ring` di Tailwind) e valgono lo stesso indaco di `accent` nei due temi. Non sono un secondo accento: non dargli un valore diverso.
- **Barra laterale scura**: `sidebar` è il fondo (`#16181d`, scuro in entrambi i temi; nello scuro è lo stesso `#12161e` di `ground`), `sidebar-text` il testo, `sidebar-text-active` la voce attiva e il marchio, `sidebar-muted` etichette e conteggi, `sidebar-active` la pillola della voce attiva e `sidebar-hover` il passaggio del mouse, cioè l’evidenza che segue il puntatore in `NavList`; `accent-on-sidebar` è l’accento schiarito per l’icona attiva, la barretta della riga corrente e il focus dentro la barra. Il suggerimento (`Tooltip`) usa gli stessi colori.
- **Zone**: `ground` è il fondo della finestra e della barra superiore (`#f6f7f9`; `#12161e` nello scuro); `surface` è il foglio su cui sta il lavoro (`#ffffff`; `#1a202c` nello scuro), e il fondo di ogni superficie sospesa. Separa con `border`, e con `border-soft` tra le righe della posta; `border-strong` per i bordi dei campi e in evidenza. `thread-open` colora le righe della conversazione aperta.
- **Testo**, in ordine di importanza: `text`, `text-secondary`, `muted`. Il corpo dei messaggi usa `body-text`; la navigazione `nav-text`.
- **Stati**: `hover` al passaggio del mouse, `selection` per ciò che è scelto, `badge` per il contatore attivo.
- **Significato**: `success` (su `success-bg`) per le conferme, `danger` su `danger-bg` e `danger-border` per errori e azioni distruttive, `priority-text` su `priority-bg`. Accompagna sempre il colore con una parola o un’icona. «Fatto» è un pulsante con il solo bordo (`border-strong`), non un riempimento verde.
- **Priorità**: ambra. Barra sinistra di 3px e pallino in `priority-mark`, sfondo appena ambrato `priority-wash`, testo `priority-text`.
- **Mittenti ed etichette**: `sender-1`…`sender-4` agli avatar, in base alla prima lettera del nome; `label-1`…`label-5` ai pallini delle etichette, in ordine; `label-bg` e `label-text` per chip e targhette (e il fondo delle schede `segment`).
- **Stampa**: `paper`, `paper-text`, `paper-muted` e `paper-rule` non cambiano con il tema.
- **Altro**: `summary-bg` e `summary-border` per il riassunto AI, `highlight` per le corrispondenze di ricerca, `scrim` e `scrim-soft` per i veli, `window-close` e `on-window-close` per il pulsante di chiusura della finestra. `avatar`, `avatar-text`, `on-accent-soft` e `on-accent-border` sono definiti, ma oggi nessuna regola li usa.

Ogni token entra in entrambi i temi quando il valore scuro differisce. Il tema scuro ha la stessa struttura del chiaro: tutto scuro, con la barra laterale e la finestra (`ground`) un gradino sotto il foglio (`surface`), e un accento schiarito con il testo scuro sopra (6,6:1 sul foglio). `border-strong` è molto più marcato nello scuro (`#6b7488`) perché i bordi dei campi restino visibili a 3:1. `node tests/tokens-compatto.mjs` fissa i valori di «Compatto» e `node tests/contrast.mjs light dark` il contrasto, misurato su coppie di colori in entrambi i temi (4,5:1 per il testo, 3:1 per segni e bordi che portano significato).

## Tipografia

Un solo carattere, **IBM Plex Sans** (licenza OFL), quello di Sigrid: i pesi 400, 500 e 600 (latino e latino esteso) più Inter per i glifi fuori dal latino, in `fonts.css` di Sigrid, che Vite impacchetta come file dell’app (l’app è offline e la CSP ha `font-src 'self'`). Qamille non porta file di caratteri propri; i pesi 550, 650 e 700 sono diventati 500 e 600. `--font-text` e `--font-title` sono alias di `--font-sans` di Sigrid. Quanto segue sul carattere variabile e sui due file woff2 in `public/fonts` è superato. Si dichiara in `system.css` come `IBM Plex Sans`, con `Segoe UI` e `system-ui` di riserva. `font-text` e `font-title` restano due token ma puntano allo stesso carattere: usa `font-text` salvo per titoli (`h1`, `h2`) e marchio, dove si scrive `font-title`.

La scala, in `text-2xs`…`text-3xl`: 11,5, 12,5, 13,5, 14, 15, 18, 20 e 26 px. La base è `text-sm` (13,5px, interlinea 1,5), impostata su `body` con il testo levigato (`-webkit-font-smoothing: antialiased`). Il corpo del messaggio è `text-md` con interlinea 1,6 e al massimo 760px di larghezza; il riassunto AI `text-md`; il titolo del messaggio (`h2`) `text-2xl`, peso 600, spaziatura -0,4px (`h1`: `text-3xl`, peso 650, -1px). Pesi: 400 per il testo, 500 per le etichette dei campi e la voce attiva della barra laterale, 600 per titoli, pulsanti e scheda scelta, 700 per il mittente di un messaggio non letto. Non scendere sotto `text-2xs`.

## Spazi e layout

- Non c’è una scala di spaziatura: copia i valori esatti dei componenti e non arrotondarli. Gli spazi più frequenti tra elementi sono 6, 7, 8, 9, 10 e 12px. I pulsanti e i campi sono alti `control-height` (32px): voci della barra laterale, pulsanti, `Select`, `DateField`, `RecipientsInput` (almeno), le schede `segment`.
- La finestra di riferimento è 1440×900: barra laterale 232px (208px sotto i 1200px), barra superiore 48px, elenco posta 360px, il resto alla lettura; a 1000×700 le stesse regole, senza scorrimento orizzontale. Il lavoro sta su un solo foglio: `surface`, 1px di `border`, angoli `radius-sheet`, staccato di 8px dal bordo destro e da quello in basso.
- Larghezze fisse dai token: `panel-narrow`, `panel-wide`, `modal-sm`, `modal-md`, `modal-lg`, e `recipients-max` per l’altezza dell’elenco completo dei destinatari.

## Bordi, ombre e raggi

- Separa con filetti da 1px, non con ombre o blocchi di colore.
- Nessuna ombra a riposo. `shadow` (un’ombra morbida e larga, `0 10px 40px`) sta solo sulle superfici fluttuanti: dialog, pannello laterale, popover, menu, liste di `Select` e dei suggerimenti, calendario di `DateField`, avvisi, composizione. `shadow-sm` esiste ma vale `none` nei due temi: i controlli sono piatti, e le regole che lo scrivono (pulsante primario, chip della scheda `segment`, marchio) non disegnano nulla. Il suggerimento non ha ombra: lo stacca il suo fondo scuro. L’unica ombra a riposo è quella piccola del pallino di Apple Switch, copiata dall’originale.
- Raggi: `radius-sm` (4px) per tag, chip, contatori, `<kbd>`, caselle, voci dei menu e suggerimento, `radius-md` (6px) per controlli, pulsanti e righe della barra laterale, `radius-lg` (8px) per popover, menu, liste, avvisi, calendario e riquadri, `radius-xl` (12px) per dialog e composizione, `radius-sheet` (14px) per il foglio, `radius-full` per pillole e interruttori.
- Livelli, in ordine: `z-popover` 10, `z-composer` 12, `z-dock` 13, `z-floating` 16, `z-panel` 20, `z-modal` 30 (il dialog centrato), `z-toast` 50 (anche le liste, il suggerimento e gli avvisi), `z-window` 100. Gli z-index da 1 a 5 restano liberi per l’ordine interno di un componente (l’evidenza di `NavList` sta a 0, le righe a 1).

## Gerarchia nelle righe

Una scelta di aspetto per la lista della posta, fatta per far capire con uno sguardo quante cose richiedono l’utente e quale leggere per prima.

- **Due pesi.** Ciò che riguarda l’utente (non letto, o nel gruppo Priorità) è scuro (`text`) e in grassetto; il resto arretra, in peso normale e colore attenuato.
- **Ciò che si chiude lascia una traccia.** Notifiche, Newsletter e Già letti partono chiusi; se un gruppo chiuso contiene non letti, l’intestazione resta scura e in grassetto, con un pallino e il numero in accento (`accent-text`, peso 700). Lo stesso per le risposte nascoste di una conversazione.
- **Le conversazioni sono righe.** Le risposte stanno nella lista una per riga, senza rientri né linee di collegamento: il legame è scritto accanto al nome («a Maria Rossi», «a te», «+N» per gli altri destinatari). Una riga non cambia mai altezza per via dei destinatari.
- **Controlli rivelati.** Le caselle di selezione compaiono al passaggio del mouse, con il focus da tastiera o se spuntate. È una scelta di aspetto: nessuno studio dice che i controlli rivelati dal mouse calmino (vedi «Evidenza e limiti»).
- **Righe senza Ark.** Le righe della posta sono markup React semplice stilato con i token, non primitive: Ark serve ai controlli, non a ogni riga, così i tempi misurati da `test:refresh`, `test:done-latency` e `test:mail-opening` non peggiorano. Entrano ed escono con un’animazione di Motion (ADR-0011 v3, decisione 6). La casella di ogni riga è un `<input type="checkbox">` nativo, l’unica eccezione di INV-003.
- **Un non letto si vede subito.** Un punto in `accent` nel margine sinistro, il mittente in grassetto e l’ora in `accent`, peso 600; le altre righe arretrano.

## Sovrapposizioni

Solo questi tre contenitori (ADR-0008): `SidePanel` per guardare o modificare in contesto, `Modal` per decidere prima di continuare, e il contenitore ancorato al suo pulsante, che ha due primitive: `Menu` per un menu di comandi, `Popover` per un filtro, un selettore o un piccolo modulo. Nessun colore di velo, larghezza o z-index letterale fuori da `theme.css`. Il pannello laterale non è modale (`aria-modal="false"`, nessuna trappola del focus, velo `scrim-soft`), il dialog centrato lo è (`aria-modal="true"`, velo `scrim`, focus trattenuto). Tutti e tre sono su Ark, hanno l’ombra `shadow` e usano la scala dei livelli; le liste (`Select`, suggerimenti), il suggerimento e gli avvisi stanno sopra i dialog, a `z-toast`. La finestra di scrittura non è un `Modal`.

## Contenuti

- **Lingua.** Ogni testo esiste in italiano e in inglese, con `tr(it, en)`. Il modulo non chiama `tr`: le parole che le primitive mostrano (`label`, `closeLabel`, `translations`, `suggestionsLabel`) le passa il chiamante.
- **Tono.** Dai del tu, in modo diretto e calmo: «I tuoi messaggi saranno qui tra un attimo.», «Scegli un contatto per vederne i dettagli.»
- **Maiuscole.** Solo la prima lettera: «Posta in arrivo», «Nuovo messaggio», «Salva modifiche».
- **Azioni.** Verbo all’imperativo, senza punto: «Rispondi», «Inoltra», «Archivia». Le voci che creano qualcosa usano il nome: «Nuovo contatto», «Nuovo evento».
- **Errori.** Di’ cosa è successo e cosa fare dopo: «Impossibile aggiornare lo stato della registrazione. Apri Meeting per verificare.»
- **Stati incompleti.** Nominali con precisione: «Trascrizione incompleta: alcuni intervalli non sono stati elaborati.»
- **Date.** Estese: l’anno resta nella data.
- **Emoji.** Mai. Gli unici segni testuali sono `↗` sui link esterni, `×` e `↑` `↓` in pochi controlli compatti.

## Stati e movimento

- Focus: contorno di 2px in `accent`, staccato di 3px (2px attorno a casella, interruttore e pomello del cursore; dentro la barra laterale è in `accent-on-sidebar` e sulle righe rientra di 2px, perché la barra scorre e lo taglierebbe). Non rimuoverlo.
- Disabilitato: opacità 0,5 e cursore normale.
- Pressione: nessun cambio di scala; i pulsanti cambiano solo al passaggio del mouse.
- Il movimento ha due forme. Le entrate in CSS durano `motion-panel` (180ms) con `ease-out` (`cubic-bezier(0.16, 1, 0.3, 1)`): il dialog sale di 8px e cresce dal 98% mentre il velo sfuma, il pannello laterale entra da destra di 24px sfumando, popover, menu e lista di `Select` scendono di 4px sfumando, il suggerimento sfuma, gli avvisi scorrono al loro posto. Le molle di Motion, che vengono dai componenti di unlumen, muovono ciò che cambia sotto gli occhi: l’evidenza di `Tabs`, l’evidenza e la barra di `NavList`, il pallino di `Switch`, le cifre di `Count`, l’entrata e l’uscita delle righe della posta.
- `prefers-reduced-motion` spegne tutto. Le entrate in CSS si spengono in ogni file (gli avvisi tengono solo la dissolvenza); `Count` mostra il numero fermo e `NavList` non mostra l’evidenza che segue il puntatore; le molle di Motion (evidenza di `Tabs`, barra di `NavItem`, pallino di `Switch`, righe delle liste) lo seguono tutte tramite `<MotionConfig reducedMotion="user">` in `desktop/src/main.tsx`.
- Il movimento accompagna un cambiamento di stato o di posizione; non aggiungere animazioni decorative.

## Marchio

Il logo è un fulmine in `on-accent` su un tassello `accent`, l’indaco: nella testata della barra laterale (`.sidebar-heading`) il tassello è di 24×24px con `radius-md` e il nome «Qamille» è in `font-title`, `text-lg`, peso 600, in `sidebar-text-active`; la regola generica `.brand-mark` (28×30px, `radius-lg`) vale fuori dalla barra. L’icona dell’app è `desktop/src-tauri/icons/icon.png`. Non ridisegnarli e non cambiarne i colori.

## Esportare e tenere allineato

Il design system in Claude e la guida Doop sono copie generate dal modulo. **Oggi le due strade sono ferme**: `npm run ds:export` (`scripts/ds-export.mjs` e `scripts/ds-export/`) e `scripts/doop-sync` sono costruiti su Svelte (montano i componenti `.svelte` in un harness Svelte e leggono le classi `svelte-*`), e vanno riscritti per React prima della prossima esportazione (ADR-0011 v3, Conseguenze). Fino ad allora non lanciarli contro il modulo React: l’aspetto è lo stesso, «Compatto», ma un’esportazione vecchia descrive i componenti Svelte e la loro struttura, non quelli React.

Quello che resta valido, e che la riscrittura deve conservare:

- `npm run ds:export` (in `desktop/`) scrive i file del design system in `desktop/ds-export/` (ignorata da git): `project/` (la cartella da pubblicare: `tokens.json`, questa guida come `README.md`, `fonts/`, `components/` con `bundle.css`, `index.d.ts` e un’anteprima e un `README.md` per componente, `assets/`, `design-system.json`), `upload/` (le icone e i loghi da caricare come asset) e `files.json`. Le anteprime delle primitive vengono dai componenti del modulo; quelle dei componenti di pagina sono catturate dall’app in esecuzione nel browser, con i dati di esempio, in italiano. È deterministico (INV-005): a parità di modulo e di dati di esempio produce gli stessi file byte per byte; `node tests/ds-export.mjs` lo prova. Per pubblicare si caricano i file di `upload/` come asset e si rilancia con `--blobs <file>` e `--at <data>`, che entrano solo in `project/design-system.json`.
- Il design system in Claude è un risultato generato: una modifica fatta sulla sua pagina va riportata nel modulo, o la prossima esportazione la sovrascrive.
- I token verso Doop: `node scripts/doop-sync/tokens.mjs export`, poi `set_guidelines` e `mark-synced`; nel verso opposto `plan` e `pull`. Le schermate: `scripts/doop-sync/extract.mjs`. Dopo ogni modifica a `theme.css` o alle schermate la guida e il prototipo Doop vanno riallineati, in entrambi i versi (`scripts/doop-sync/README.md`).
- Le immagini di riferimento di `visual-snapshot.mjs` e `visual-compare.mjs` misurano «Compatto» sui componenti Svelte: i valori restano, ma vanno riconfrontate per le piccole differenze di movimento e di struttura (la sottolineatura di `Tabs` è un’evidenza scorrevole, `Switch` è un `<button role="switch">`) e rigenerate dopo l’approvazione degli screenshot nei due temi.

## Evidenza e limiti

Un controllo di letteratura (17 ricerche, 8 articoli letti, 7 a testo pieno, fatto da un agente di ricerca il 2026-11-07, non pubblicato in `docs/research/`) non ha trovato **nessuno studio su interfacce con adulti con ADHD** per nessuna delle scelte di questa guida. Quello che esiste è psicologia di laboratorio; il passaggio al design è inferenza. Le scelte di aspetto qui sopra, compresi i componenti di unlumen e il loro movimento, sono scelte di aspetto e di densità, non effetti dimostrati.

Quello che le fonti permettono di dire, e solo come inferenza:

- **Salienza.** Zhang, Abagis, Steeby, Jonides 2024 (*Visual Cognition*; 36 adulti con ADHD, 46 senza): la prima cattura dello sguardo da parte di un elemento fuori posto è uguale nei due gruppi (39% contro 40%); chi ha ADHD ci resta più a lungo (seconda fissazione 30,7% contro 24,7%). Forster et al. 2014 (*Neuropsychology*; 17 contro 16): il costo di un distrattore raro era maggiore anche a carico alto. Inferenza: pochi elementi evidenti e utili, non decorazione; per questo un solo accento, usato poco.
- **Meno non è sempre meglio.** Lo stesso risultato non permette di assumere che «più vuoto» aiuti sempre.
- **Ciò che si chiude.** Nessun esperimento controllato su controlli rivelati al passaggio del mouse o su elementi chiusi. Fuermaier et al. 2013 (*PLoS One*): nella memoria prospettica gli adulti con ADHD ricordano bene il piano ma pianificano e passano da un compito all’altro peggio. Inferenza: ciò che si chiude deve lasciare una traccia visibile; per questo i conteggi in accento sui gruppi chiusi.
- **Carattere.** Wallace et al. 2022 (*ACM TOCHI*, 352 adulti): il carattere più veloce per una persona è molto più veloce del più lento, ma non c’è un effetto medio, e chi dichiarava condizioni di lettura o neurologiche era escluso. **Non esiste un test indipendente su un carattere «per l’ADHD»**: IBM Plex Sans è una scelta di aspetto. Una strada più solida, per dopo, è un’impostazione per scegliere il carattere.
- **Anteprima e smistamento.** Weaver et al. 2011 (MobileHCI, 16 persone, mobile): 2–3 righe di anteprima riducono i passaggi tra panoramica e dettaglio. Sarrafzadeh et al. 2019 (CHIIR): lo smistamento dipende soprattutto dall’importanza del lavoro, non solo dal mittente. Nessuno studio controllato sui riassunti AI.
- **Movimento.** Nessuno studio trovato. Le molle dei componenti di unlumen sono una scelta di aspetto; resta il principio di `docs/ricerca-email-specifica.md` §1.3.5, niente animazioni decorative, e la regola di rispettare `prefers-reduced-motion`.

Cose che questa guida **non** afferma: «un carattere speciale aiuta l’ADHD», «gli adulti con ADHD sono catturati di più dai colori vivi», «i controlli rivelati dal mouse calmano», «il movimento a molla aiuta a seguire i cambiamenti», «il mittente decide lo smistamento». Chi aggiunge a questa pagina un’affermazione di efficacia deve citare uno studio che la sostiene.

## Invarianti (ADR-0011 v3)

La numerazione è quella dell’ADR-0011 v3, approvato il 2026-11-12. La v3 cambia INV-001 e INV-006, aggiunge INV-008 e lascia gli altri come nella v2; `npm run build` esegue `test:ds` (`ds-usage.mjs`, `style-tokens.mjs`, `tokens-compatto.mjs`, `contrast.mjs light dark`) dopo `tsc`.

- **INV-001** (cambiato) I file in `desktop/src/ds/` non importano da `desktop/src/lib/`, da `desktop/src/components/` né da `@tauri-apps`; i componenti di unlumen copiati stanno in `desktop/src/ds/unlumen/` e importano solo dal modulo.
- **INV-002** Fuori da `desktop/src/ds/`, il markup non usa le classi `primary`, `secondary`, `icon-button`, `text-button`, `popover`, `side-panel-layer` né l’elemento `<dialog>`. Un nuovo `<button className="primary">` in `desktop/src/components/` fa fallire `node tests/ds-usage.mjs`, che indica file, riga e primitiva da usare.
- **INV-003** Fuori da `desktop/src/ds/`, il markup non usa `<select>`, `<input type="checkbox">` né `role="tablist"` scritto a mano. Eccezione: la casella di selezione di ogni riga della posta in `components/MailList.tsx`, con il motivo «rows stay plain markup (spec 7.6)». `node tests/ds-usage.mjs` lo controlla, leggendo i `.tsx` con il parser di TypeScript.
- **INV-004** I valori dei token stanno solo in `desktop/src/ds/tokens/theme.css`; `node tests/style-tokens.mjs` lo controlla su `.css`, `.ts` e `.tsx`, e lascia fuori `ds/unlumen/` (codice copiato così com’è).
- **INV-005** `npm run ds:export` è deterministico. Vale per l’esportazione riscritta per React; quella di oggi è su Svelte (vedi «Esportare e tenere allineato»).
- **INV-006** (cambiato) Il modulo contiene una sola implementazione di ogni componente, in React.
- **INV-007** Fuori da `desktop/src/ds/` nessun file importa da `@ark-ui/*`: i componenti passano da una primitiva. `node tests/ds-usage.mjs` lo controlla.
- **INV-008** (nuovo) I componenti di unlumen non portano colori propri: leggono i token di `theme.css`, direttamente con `var(--…)` o tramite i nomi di Tailwind di `styles/tailwind.css`. `node tests/tokens-compatto.mjs` (i valori di «Compatto») e `node tests/contrast.mjs light dark` continuano a valere; dentro `ds/unlumen/` il controllo è a mano (vedi «I componenti di unlumen»).
