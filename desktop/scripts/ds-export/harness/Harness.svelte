<script lang="ts">
  // The primitives of desktop/src/ds, mounted as the app mounts them (ADR-0011). The export reads this
  // page's DOM: each case is found by its data-case, and a forced state is a data-state on the control,
  // which bundle.css maps to the real :hover and :focus-visible rules. The Ark parts that exist only while
  // open (the popover, the tooltip, the lists, the calendar, the notices) are opened by capture-primitives.mjs,
  // the way a person would: it clicks, hovers and types, then takes the part from <body>.
  import Button from '../../../src/ds/primitives/Button.svelte';
  import Checkbox from '../../../src/ds/primitives/Checkbox.svelte';
  import DateField from '../../../src/ds/primitives/DateField.svelte';
  import Field from '../../../src/ds/primitives/Field.svelte';
  import Icon from '../../../src/ds/primitives/Icon.svelte';
  import Modal from '../../../src/ds/primitives/Modal.svelte';
  import Popover from '../../../src/ds/primitives/Popover.svelte';
  import RecipientsInput from '../../../src/ds/primitives/RecipientsInput.svelte';
  import Select from '../../../src/ds/primitives/Select.svelte';
  import SidePanel from '../../../src/ds/primitives/SidePanel.svelte';
  import Slider from '../../../src/ds/primitives/Slider.svelte';
  import Switch from '../../../src/ds/primitives/Switch.svelte';
  import Tabs from '../../../src/ds/primitives/Tabs.svelte';
  import Toaster, { notify } from '../../../src/ds/primitives/Toaster.svelte';

  // The icon names come from the exporter, which reads them from Icon.svelte.
  const icons: string[] = (window as unknown as { __DS_ICONS__?: string[] }).__DS_ICONS__ ?? [];
  // The notices are raised from outside, by the capture (a notice lives a few seconds).
  (window as unknown as { __dsNotify: typeof notify }).__dsNotify = notify;

  // One dialog or panel at a time: the layers of Ark count each other (data-has-nested, --layer-index), and a
  // modal covers the page. The capture asks for each by name ('modal-sm', 'side-panel-wide'...) and then for none.
  let overlay = $state('');
  (window as unknown as { __dsOverlay: (name: string) => void }).__dsOverlay = (name) => (overlay = name);

  const variants = ['primary', 'secondary', 'icon', 'text'] as const;
  const states = [
    ['default', 'a riposo'],
    ['hover', 'al passaggio del mouse'],
    ['focus', 'con il focus da tastiera'],
    ['disabled', 'disattivato'],
  ] as const;
  const forced = (state: string) => (state === 'hover' || state === 'focus' ? state : undefined);

  // The words of the calendar, as the app gives them (Italian).
  const calendarWords = {
    trigger: (open: boolean) => (open ? 'Chiudi il calendario' : 'Apri il calendario'),
    content: 'Calendario',
    nextTrigger: (view: string) => (view === 'day' ? 'Mese successivo' : view === 'month' ? 'Anno successivo' : 'Pagina successiva'),
    prevTrigger: (view: string) => (view === 'day' ? 'Mese precedente' : view === 'month' ? 'Anno precedente' : 'Pagina precedente'),
    viewTrigger: (view: string) => (view === 'day' ? 'Scegli il mese' : view === 'month' ? 'Scegli l’anno' : 'Scegli il giorno'),
    clearTrigger: 'Cancella la data',
    placeholder: () => ({ day: 'gg', month: 'mm', year: 'aaaa' }),
  };

  const calendars = [
    { value: 'work', label: 'Lavoro' },
    { value: 'personal', label: 'Personale' },
    { value: 'team', label: 'Team design' },
  ];
  let calendarClosed = $state('work');
  let calendarOpen = $state('work');
  let calendarEmpty = $state('');

  let popoverOpen = $state(false);
  let tab = $state('inbox');
  let segment = $state('week');
  let slider = $state(40);
  let ticked = $state(true);
  let unticked = $state(false);
  let part = $state(false);
  let on = $state(true);
  let off = $state(false);
  let day = $state('2026-11-03');
  let bounded = $state('');

  // The people of the recipients case: two chips, one of them refused, and a short address book.
  let people = $state(['Maria Rossi <maria.rossi@studio-example.com>', 'giorgio@studio-example']);
  let pending = $state('');
  const book = [
    { email: 'marco.bianchi@studio-example.com', name: 'Marco Bianchi' },
    { email: 'martina.neri@studio-example.com', name: 'Martina Neri' },
    { email: 'luca.verdi@studio-example.com', name: 'Luca Verdi' },
  ];
  let suggestions = $state<typeof book>([]);
  const ask = (text: string) => {
    const q = text.trim().toLowerCase();
    suggestions = q ? book.filter((p) => p.name.toLowerCase().includes(q) || p.email.includes(q)) : [];
  };
</script>

<main>
  {#each variants as variant (variant)}
    {#each states as [state, label] (state)}
      <figure class="ds-case" data-case="button-{variant}-{state}">
        {#if variant === 'primary'}
          <Button {variant} data-state={forced(state)} disabled={state === 'disabled'}><Icon name="reply" size={16} />Rispondi</Button>
        {:else if variant === 'secondary'}
          <Button {variant} data-state={forced(state)} disabled={state === 'disabled'}>Annulla</Button>
        {:else if variant === 'icon'}
          <Button {variant} data-state={forced(state)} disabled={state === 'disabled'} aria-label="Archivia"><Icon name="archive" size={18} /></Button>
        {:else}
          <Button {variant} data-state={forced(state)} disabled={state === 'disabled'}>Mostra altri filtri</Button>
        {/if}
        <figcaption class="ds-caption">{variant}, {label}</figcaption>
      </figure>
    {/each}
  {/each}

  <figure class="ds-case ds-field" data-case="field-text">
    <Field><span>Nome</span><input value="Alex B." /></Field>
    <figcaption class="ds-caption">campo di testo</figcaption>
  </figure>
  <figure class="ds-case ds-field" data-case="field-focus">
    <Field><span>Oggetto</span><input data-state="focus" value="Riunione di giovedì" /></Field>
    <figcaption class="ds-caption">con il focus da tastiera</figcaption>
  </figure>
  <figure class="ds-case ds-field" data-case="field-disabled">
    <Field><span>Account</span><input value="alex@studio-example.com" disabled /></Field>
    <figcaption class="ds-caption">disattivato</figcaption>
  </figure>
  <figure class="ds-case ds-field" data-case="field-help">
    <Field><span>Firma</span><input value="Alex B., Studio Example" /></Field>
    <p class="field-help">Il testo di aiuto sta sotto il campo.</p>
    <figcaption class="ds-caption">con testo di aiuto</figcaption>
  </figure>
  <figure class="ds-case ds-field" data-case="field-textarea">
    <Field><span>Note</span><textarea rows="3">Portare il budget aggiornato.</textarea></Field>
    <figcaption class="ds-caption">testo su più righe</figcaption>
  </figure>

  <!-- A Modal opens itself over everything, in <body>; the export takes the content of each. -->
  {#if overlay === 'modal-sm'}
  <Modal size="sm" data-case="modal-sm" aria-label="Elimina contatto">
    <h2>Eliminare il contatto?</h2>
    <p class="ds-modal-text">Maria Rossi sparirà da tutte le rubriche.</p>
    <div class="modal-actions">
      <Button variant="secondary">Annulla</Button>
      <Button variant="primary">Elimina</Button>
    </div>
  </Modal>
  {:else if overlay === 'modal-md'}
  <Modal data-case="modal-md" aria-label="Unisci contatti">
    <h2>Unisci contatti</h2>
    <p class="ds-modal-text">Il contatto unito conserva tutti gli indirizzi e i numeri dei due.</p>
    <div class="modal-actions">
      <Button variant="secondary">Annulla</Button>
      <Button variant="primary">Unisci</Button>
    </div>
  </Modal>
  {:else if overlay === 'modal-lg'}
  <Modal size="lg" data-case="modal-lg" aria-label="Anteprima di stampa">
    <h2>Anteprima di stampa</h2>
    <p class="ds-modal-text">Due messaggi selezionati, su carta bianca in entrambi i temi.</p>
    <div class="modal-actions">
      <Button variant="secondary">Chiudi</Button>
      <Button variant="primary">Stampa</Button>
    </div>
  </Modal>
  {/if}

  <!-- The layer fills its nearest positioned ancestor, so each panel gets a stage of its own. -->
  <div class="ds-stage" data-case="side-panel-wide">
    {#if overlay === 'side-panel-wide'}
    <SidePanel onscrim={() => {}} aria-label="Allegato">
      <div class="side-panel-bar">
        <strong class="panel-title">Q4-proposal.txt</strong>
        <Button variant="icon" class="panel-close" aria-label="Chiudi l’allegato"><Icon name="close" size={16} /></Button>
      </div>
      <div class="side-panel-body"><p class="ds-body">Il contenuto dell’allegato, che scorre.</p></div>
    </SidePanel>
    {/if}
  </div>
  <div class="ds-stage" data-case="side-panel-narrow">
    {#if overlay === 'side-panel-narrow'}
    <SidePanel narrow onscrim={() => {}} aria-label="Evento">
      <div class="side-panel-bar">
        <strong class="panel-title">Design review</strong>
        <Button variant="icon" class="panel-close" aria-label="Chiudi"><Icon name="close" size={16} /></Button>
      </div>
      <div class="side-panel-body"><p class="ds-body">Il dettaglio dell’evento, nello stretto.</p></div>
    </SidePanel>
    {/if}
  </div>

  <!-- The popover exists only while open: the capture clicks its button. -->
  <figure class="ds-case" data-case="popover">
    <Popover bind:open={popoverOpen} label="Altre azioni sul messaggio">
      {#snippet trigger(props)}
        <Button variant="icon" aria-label="Altre azioni sul messaggio" {...props}><Icon name="more" size={18} /></Button>
      {/snippet}
      <button><Icon name="print" size={16} />Stampa email</button>
      <button><Icon name="download" size={16} />Scarica .eml</button>
      <span class="popover-divider"></span>
      <button><Icon name="trash" size={16} />Elimina</button>
    </Popover>
    <figcaption class="ds-caption">il pulsante che lo apre</figcaption>
  </figure>

  <!-- The tooltip: a hint of an icon button, with its real shortcut. The capture hovers the button. -->
  <figure class="ds-case" data-case="tooltip">
    <Button variant="icon" aria-label="Rispondi" tip="Rispondi" shortcut="R"><Icon name="reply" size={18} /></Button>
    <figcaption class="ds-caption">il pulsante che lo mostra</figcaption>
  </figure>

  <figure class="ds-case ds-field" data-case="select-closed">
    <Select bind:value={calendarClosed} items={calendars} label="Calendario" />
    <figcaption class="ds-caption">a riposo, con l’etichetta</figcaption>
  </figure>
  <figure class="ds-case ds-field" data-case="select-open">
    <Select bind:value={calendarOpen} items={calendars} label="Calendario" />
    <figcaption class="ds-caption">aperto, con la scelta evidenziata</figcaption>
  </figure>
  <figure class="ds-case ds-field" data-case="select-empty">
    <Select bind:value={calendarEmpty} items={calendars} aria-label="Calendario" placeholder="Scegli un calendario" />
    <figcaption class="ds-caption">senza scelta, con il suggerimento</figcaption>
  </figure>
  <figure class="ds-case ds-field" data-case="select-disabled">
    <Select value="work" items={calendars} aria-label="Calendario" disabled />
    <figcaption class="ds-caption">disattivato</figcaption>
  </figure>

  <figure class="ds-case" data-case="checkbox-off"><Checkbox bind:checked={unticked}>Segna come letto</Checkbox><figcaption class="ds-caption">non spuntata</figcaption></figure>
  <figure class="ds-case" data-case="checkbox-on"><Checkbox bind:checked={ticked}>Segna come letto</Checkbox><figcaption class="ds-caption">spuntata</figcaption></figure>
  <figure class="ds-case" data-case="checkbox-mixed"><Checkbox bind:checked={part} indeterminate>Tutti i messaggi</Checkbox><figcaption class="ds-caption">alcuni, non tutti</figcaption></figure>
  <figure class="ds-case" data-case="checkbox-disabled"><Checkbox checked={false} disabled>Segna come letto</Checkbox><figcaption class="ds-caption">disattivata</figcaption></figure>

  <figure class="ds-case" data-case="switch-on"><Switch bind:checked={on}>Notifiche sul desktop</Switch><figcaption class="ds-caption">acceso</figcaption></figure>
  <figure class="ds-case" data-case="switch-off"><Switch bind:checked={off}>Notifiche sul desktop</Switch><figcaption class="ds-caption">spento</figcaption></figure>
  <figure class="ds-case" data-case="switch-disabled"><Switch checked={false} disabled>Notifiche sul desktop</Switch><figcaption class="ds-caption">disattivato</figcaption></figure>

  <figure class="ds-case ds-wide" data-case="tabs-underline">
    <Tabs
      bind:value={tab}
      aria-label="Posta"
      items={[
        { value: 'inbox', label: 'Ricevuti', count: 12 },
        { value: 'sent', label: 'Inviati' },
        { value: 'drafts', label: 'Bozze', count: 3 },
      ]}
    />
    <figcaption class="ds-caption">sottolineata, con il conteggio</figcaption>
  </figure>
  <figure class="ds-case ds-wide" data-case="tabs-segment">
    <Tabs
      bind:value={segment}
      variant="segment"
      aria-label="Vista"
      items={[
        { value: 'day', label: 'Giorno' },
        { value: 'week', label: 'Settimana' },
        { value: 'month', label: 'Mese' },
      ]}
    />
    <figcaption class="ds-caption">a segmenti</figcaption>
  </figure>

  <figure class="ds-case ds-field" data-case="slider">
    <Slider bind:value={slider} min={0} max={100} aria-label="Posizione" />
    <figcaption class="ds-caption">cursore a un solo valore</figcaption>
  </figure>

  <figure class="ds-case ds-field" data-case="datefield-closed">
    <DateField bind:value={day} aria-label="Data di inizio" locale="it-IT" translations={calendarWords} />
    <figcaption class="ds-caption">con una data</figcaption>
  </figure>
  <figure class="ds-case ds-field" data-case="datefield-open">
    <DateField bind:value={bounded} aria-label="Data di fine" locale="it-IT" translations={calendarWords} min="2026-11-02" max="2026-11-27" />
    <figcaption class="ds-caption">vuoto, con il calendario aperto</figcaption>
  </figure>

  <figure class="ds-case ds-recipients" data-case="recipients">
    <RecipientsInput
      bind:value={people}
      bind:pending
      {suggestions}
      aria-label="Destinatari"
      placeholder="Aggiungi un destinatario"
      suggestions-label="Suggerimenti dalla rubrica"
      onquery={ask}
      invalid={(email) => !/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email)}
    />
    <figcaption class="ds-caption">chip, uno non valido, e i suggerimenti dalla rubrica</figcaption>
  </figure>

  <Toaster label="Notifiche" closeLabel="Chiudi la notifica" />

  <div class="ds-wrap ds-icons" data-case="icons">
    {#each icons as name (name)}
      <figure class="ds-icon-cell">
        <Icon {name} size={20} />
        <figcaption class="ds-caption">{name}</figcaption>
      </figure>
    {/each}
  </div>
</main>
