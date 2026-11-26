// Pagina /admin di qamille-server.
//
// Regole: la chiave amministratore sta solo in una variabile di questa pagina e va
// in Authorization; nessun cookie, nessun salvataggio nel browser. I nodi si creano
// con createElement e textContent, mai interpretando testo come HTML. Un 401
// svuota la chiave e la richiede.
(() => {
  'use strict';

  let adminKey = null;
  // «Esci» (o una chiave rifiutata) chiude la sessione: una risposta partita prima non ridisegna nulla.
  let session = 0;
  let defaultMax = 0;
  // Gli utenti dell'ultimo caricamento: aprire o chiudere una riga, o annullare, li ridisegna senza rete.
  let lastUsers = [];
  let llmOn = false;
  const openDetails = new Set();

  const byId = (id) => document.getElementById(id);

  function el(tag, options, ...children) {
    const node = document.createElement(tag);
    const opts = options || {};
    for (const [name, value] of Object.entries(opts)) {
      if (value === undefined || value === null || value === false) continue;
      if (name === 'text') node.textContent = value;
      else if (name === 'className') node.className = value;
      else node.setAttribute(name, value === true ? '' : String(value));
    }
    for (const child of children) {
      if (child !== null && child !== undefined) node.append(child);
    }
    return node;
  }

  function clear(node) {
    while (node.firstChild) node.removeChild(node.firstChild);
  }

  function formatDate(value) {
    if (!value) return '—';
    const date = new Date(value);
    return Number.isNaN(date.getTime()) ? value : date.toLocaleString('it-IT');
  }

  class Unauthorized extends Error {}

  async function api(method, path, body) {
    if (!adminKey) throw new Unauthorized('chiave mancante');
    const headers = { Authorization: 'Bearer ' + adminKey };
    const init = { method, headers, cache: 'no-store', credentials: 'omit', redirect: 'error' };
    if (body !== undefined) {
      headers['Content-Type'] = 'application/json';
      init.body = JSON.stringify(body);
    }
    const mine = session;
    let response;
    try {
      response = await fetch('/admin/api' + path, init);
    } catch (e) {
      if (mine !== session) throw new Unauthorized('sessione chiusa');
      throw new Error('Il server non risponde.');
    }
    if (mine !== session) throw new Unauthorized('sessione chiusa');
    if (response.status === 401) {
      lock('La chiave non è valida: scrivila di nuovo.');
      throw new Unauthorized('401');
    }
    if (response.status === 204) return null;
    let data = null;
    try {
      data = await response.json();
    } catch (e) {
      data = null;
    }
    if (mine !== session) throw new Unauthorized('sessione chiusa');
    if (!response.ok) {
      const message = data && data.error && data.error.message;
      const failure = new Error(message || 'Errore ' + response.status + '.');
      failure.status = response.status;
      throw failure;
    }
    return data;
  }

  // --- Sezioni: una alla volta, scelta dalla barra ------------------------------------------

  function showView(name) {
    for (const section of document.querySelectorAll('.view')) {
      section.classList.toggle('current', section.id === name + '-section');
    }
    for (const item of document.querySelectorAll('.nav-item[data-view]')) {
      if (item.getAttribute('data-view') === name) item.setAttribute('aria-current', 'page');
      else item.removeAttribute('aria-current');
    }
  }

  // Una voce della barra c'è solo se la sua sezione c'è su questo server.
  function syncNav() {
    for (const item of document.querySelectorAll('.nav-item[data-view]')) {
      item.hidden = byId(item.getAttribute('data-view') + '-section').hidden;
    }
    const current = document.querySelector('.view.current');
    if (!current || current.hidden) showView('users');
  }

  // Un avviso d'errore: vuoto, sparisce.
  function note(id, message) {
    const node = byId(id);
    node.textContent = message || '';
    node.hidden = !message;
  }

  function setStatus(message) {
    byId('status').textContent = message || '';
  }

  function setError(message) {
    note('error', message);
  }

  function hideSecret() {
    byId('secret-value').textContent = '';
    byId('secret-label').textContent = '';
    byId('secret-copy').textContent = 'Copia';
    byId('secret-box').hidden = true;
  }

  function showSecret(label, value) {
    byId('secret-label').textContent = label;
    byId('secret-value').textContent = value;
    byId('secret-copy').textContent = 'Copia';
    byId('secret-box').hidden = false;
    byId('secret-copy').focus();
  }

  function lock(message) {
    adminKey = null;
    session += 1;
    lastUsers = [];
    // Un accesso ancora in corso non tiene occupato «Entra»: la sua risposta non conta più.
    byId('key-submit').disabled = false;
    byId('key-submit').removeAttribute('aria-busy');
    openDetails.clear();
    hideSecret();
    clear(byId('users-body'));
    byId('users-section').hidden = true;
    byId('models-section').hidden = true;
    byId('llm-section').hidden = true;
    byId('embedding-section').hidden = true;
    byId('llm-key').value = '';
    byId('embedding-key').value = '';
    setLlmStatus('');
    setLlmError('');
    lastEmbedding = null;
    disarmAll();
    setEmbeddingStatus('');
    setEmbeddingError('');
    clearTimeout(modelsTimer);
    modelsTimer = null;
    clear(byId('models-body'));
    setModelsStatus('');
    setModelsError('');
    byId('lock').hidden = true;
    byId('shell').hidden = true;
    byId('key-section').hidden = false;
    byId('key-input').value = '';
    note('key-error', message);
    setStatus('');
    setError('');
    byId('key-input').focus();
  }

  // Un'azione con il suo pulsante occupato finché dura; l'errore va nell'avviso della sezione.
  function runner(report, before) {
    return async (button, action) => {
      if (button) {
        button.disabled = true;
        button.setAttribute('aria-busy', 'true');
      }
      report('');
      if (before) before();
      try {
        await action();
      } catch (e) {
        if (!(e instanceof Unauthorized)) report(e.message);
      } finally {
        if (button) {
          button.disabled = false;
          button.removeAttribute('aria-busy');
        }
      }
    };
  }

  const run = runner(setError, () => setStatus(''));
  // Alla chiave: l'errore si legge lì, la pagina dietro non c'è ancora.
  const runKey = runner((message) => note('key-error', message));

  // Un'azione che costa chiede un secondo clic sullo stesso pulsante. Il primo lo arma e lo dice;
  // Esc, o il fuoco che va altrove, lo disarmano. Vero quando il clic è il secondo.
  const armedAt = new WeakMap();

  // Il pulsante di un'azione che costa: l'etichetta di conferma è già dentro, nascosta, così armarlo
  // non ne cambia la misura né sposta nulla, e il secondo clic cade dove è caduto il primo.
  function guardedButton(label, armedLabel, className) {
    return el('button', { type: 'button', className: (className ? className + ' ' : '') + 'guarded' },
      el('span', { className: 'when-plain', text: label }),
      el('span', { className: 'when-armed', text: armedLabel }));
  }

  function secondClick(button, what) {
    // Un doppio clic è due clic in pochi millisecondi: il secondo conta solo dopo mezzo secondo.
    if (button.classList.contains('confirm') && Date.now() - armedAt.get(button) < 500) return false;
    if (disarm(button)) return true;
    armedAt.set(button, Date.now());
    button.classList.add('confirm');
    // Che cosa succederà: al passaggio del mouse e a chi legge la pagina con la voce, senza
    // aggiungere righe che sposterebbero il pulsante.
    const hint = what + ' Premi di nuovo per confermare, Esc per annullare.';
    button.setAttribute('title', hint);
    byId('armed-note').textContent = hint;
    button.addEventListener('blur', () => disarm(button), { once: true });
    return false;
  }

  // Tutti i pulsanti armati tranne quello dato: un clic altrove, Esc, «Esci».
  function disarmAll(except) {
    for (const button of document.querySelectorAll('button.confirm')) {
      if (button !== except) disarm(button);
    }
  }

  // Vero se il pulsante era armato.
  function disarm(button) {
    if (!button.classList.contains('confirm')) return false;
    button.classList.remove('confirm');
    button.removeAttribute('title');
    byId('armed-note').textContent = '';
    return true;
  }

  function stateOf(user) {
    if (user.disabled_at) return { text: 'Disattivato', className: 'pill-danger' };
    if (user.password_temporary) {
      const expired = user.temporary_expires_at && new Date(user.temporary_expires_at) <= new Date();
      return {
        text: expired ? 'Provvisoria scaduta' : 'Password provvisoria',
        className: 'pill-warn',
      };
    }
    return { text: 'Attivo', className: 'pill-ok' };
  }

  function llmTokensCell(user) {
    const usage = user.usage || {};
    const incoming = usage.llm_prompt_tokens || 0;
    const outgoing = usage.llm_completion_tokens || 0;
    return el('td', {
      text: (incoming + outgoing).toLocaleString('it-IT'),
      className: 'num',
      'data-label': 'Token',
    });
  }

  function llmTokensText(user) {
    const usage = user.usage || {};
    return (usage.llm_prompt_tokens || 0).toLocaleString('it-IT') + ' in entrata, ' +
      (usage.llm_completion_tokens || 0).toLocaleString('it-IT') + ' in uscita';
  }

  function maxText(user) {
    const label = (user.max_devices ?? defaultMax) === 0 ? 'senza limite' : String(user.max_devices ?? defaultMax);
    return user.max_devices == null ? label + ' (predefinito)' : label;
  }

  // «2 di 5», o «2» dove non c'è un massimo.
  function devicesText(user) {
    const max = user.max_devices ?? defaultMax;
    return max === 0 ? String(user.active_devices) : user.active_devices + ' di ' + max;
  }

  // Le colonne della tabella degli utenti, per la riga aperta sotto un utente.
  function columns() {
    return 4 + (llmOn ? 1 : 0);
  }

  async function loadUsers() {
    const data = await api('GET', '/users');
    defaultMax = data.default_max_devices;
    llmOn = Boolean(data.llm);
    byId('llm-col').hidden = !data.llm;
    renderUsers(data.users);
    byId('key-section').hidden = true;
    byId('users-section').hidden = false;
    byId('shell').hidden = false;
    byId('lock').hidden = false;
  }

  // Dopo la chiave: gli utenti, poi le sezioni che questo server ha. Una sezione che non si carica
  // resta, con il suo errore e il suo «Aggiorna».
  async function loadAll() {
    await loadUsers();
    syncNav();
    byId('content').focus();
    await loadModels();
    await loadLlm();
    await loadEmbedding();
    syncNav();
  }

  function renderUsers(users) {
    lastUsers = users;
    const body = byId('users-body');
    clear(body);
    byId('empty').hidden = users.length > 0;
    byId('users-wrap').hidden = users.length === 0;
    byId('users-count').textContent = users.length === 1 ? '1 utente' : users.length + ' utenti';
    for (const user of users) {
      const state = stateOf(user);
      const open = openDetails.has(user.id);
      const manage = el('button', {
        type: 'button',
        className: 'secondary',
        text: open ? 'Chiudi' : 'Gestisci',
        'aria-expanded': open ? 'true' : 'false',
      });
      manage.addEventListener('click', () => {
        if (openDetails.has(user.id)) openDetails.delete(user.id);
        else openDetails.add(user.id);
        redrawUsers(user);
      });
      body.append(el('tr', { 'data-user': user.username, className: open ? 'open' : null },
        el('td', {},
          el('strong', { text: user.username }),
          user.email ? el('span', { className: 'sub', text: user.email }) : null),
        el('td', {}, el('span', { className: 'pill ' + state.className, text: state.text })),
        el('td', { text: devicesText(user), className: 'num', 'data-label': 'Dispositivi' }),
        llmOn ? llmTokensCell(user) : null,
        el('td', { className: 'col-actions' }, manage)));
      if (open) {
        const details = el('tr', { className: 'details' });
        body.append(details);
        renderDetails(user, details);
      }
    }
  }

  // La tabella di prima, ridisegnata (una riga aperta o chiusa), con il fuoco sulla riga dell'utente.
  function redrawUsers(user) {
    renderUsers(lastUsers);
    focusUser(user.username);
  }

  // Dopo un'azione su un utente: la tabella aggiornata, con il fuoco di nuovo sulla sua riga.
  async function reloadUsers(user) {
    await loadUsers();
    focusUser(user.username);
  }

  // La tabella si ridisegna a ogni azione: il fuoco torna sul pulsante dell'utente.
  function focusUser(username) {
    for (const row of byId('users-body').children) {
      if (row.getAttribute('data-user') === username) {
        const button = row.querySelector('button');
        if (button) button.focus();
      }
    }
  }

  // Sotto un utente aperto: i suoi dispositivi e le azioni sull'account.
  function renderDetails(user, row) {
    const devices = el('div', { className: 'panel-block' }, el('h2', { text: 'Dispositivi' }));
    const account = el('div', { className: 'panel-block' },
      el('h2', { text: 'Account' }),
      el('p', { className: 'hint', text: 'Massimo di dispositivi: ' + maxText(user) + '.' }),
      llmOn ? el('p', { className: 'hint', text: 'Token assistente: ' + llmTokensText(user) + '.' }) : null,
      rowActions(user));
    row.append(el('td', { colspan: String(columns()) }, el('div', { className: 'panel' }, devices, account)));
    renderDevices(user, devices);
  }

  function rowActions(user) {
    const box = el('div', { className: 'row-actions' });

    const reset = guardedButton('Reimposta password', 'Sì, reimposta', 'secondary');
    reset.addEventListener('click', () => {
      if (!secondClick(reset, 'Una nuova password provvisoria per «' + user.username + '»; i suoi dispositivi vengono disconnessi.')) return;
      run(reset, async () => {
        const result = await api('POST', '/users/' + encodeURIComponent(user.id) + '/reset-password');
        // Prima la password, che si vede una volta sola: la tabella può non ricaricarsi.
        showSecret(
          'Nuova password provvisoria per «' + user.username + '», valida fino a ' +
            formatDate(result.temporary_expires_at) + ' (i dispositivi sono stati disconnessi):',
          result.temporary_password);
        await loadUsers();
      });
    });

    const toggle = user.disabled_at
      ? el('button', { type: 'button', className: 'secondary', text: 'Riattiva' })
      : guardedButton('Disattiva', 'Sì, disattiva', 'danger');
    const disconnects = 'Disattivando «' + user.username + '» disconnetti tutti i suoi dispositivi. Dopo la riattivazione servirà un nuovo accesso su ciascuno.';
    toggle.addEventListener('click', () => {
      if (!user.disabled_at && !secondClick(toggle, disconnects)) return;
      run(toggle, async () => {
        const action = user.disabled_at ? 'enable' : 'disable';
        await api('POST', '/users/' + encodeURIComponent(user.id) + '/' + action);
        await reloadUsers(user);
        setStatus(user.disabled_at ? '«' + user.username + '» riattivato.' : '«' + user.username + '» disattivato.');
      });
    });

    const max = el('button', { type: 'button', className: 'secondary', text: 'Massimo di dispositivi…' });
    max.addEventListener('click', () => {
      box.replaceChildren(maxEditor(user));
    });

    const remove = guardedButton('Cancella', 'Sì, cancella', 'danger');
    remove.addEventListener('click', () => {
      if (!secondClick(remove, 'Cancella «' + user.username + '» con i suoi dispositivi.')) return;
      run(remove, async () => {
        await api('DELETE', '/users/' + encodeURIComponent(user.id));
        openDetails.delete(user.id);
        await loadUsers();
        byId('content').focus();
        setStatus('«' + user.username + '» cancellato con i suoi dispositivi.');
      });
    });

    box.append(reset, max, toggle, remove);
    if (!user.disabled_at) box.append(el('p', { className: 'hint', text: disconnects }));
    return box;
  }

  function maxEditor(user) {
    const input = el('input', {
      type: 'number',
      min: '0',
      step: '1',
      inputmode: 'numeric',
      'aria-label': 'Massimo di dispositivi per ' + user.username,
      placeholder: String(defaultMax),
    });
    if (user.max_devices !== null && user.max_devices !== undefined) input.value = String(user.max_devices);
    const save = el('button', { type: 'submit', text: 'Salva' });
    const useDefault = el('button', { type: 'button', className: 'secondary', text: 'Predefinito' });
    const cancel = el('button', { type: 'button', className: 'secondary', text: 'Annulla' });
    const submit = (button, value) => run(button, async () => {
      await api('POST', '/users/' + encodeURIComponent(user.id) + '/max-devices', { max_devices: value });
      await reloadUsers(user);
      setStatus('Massimo di dispositivi aggiornato per «' + user.username + '».');
    });
    const back = () => redrawUsers(user);
    useDefault.addEventListener('click', () => submit(useDefault, null));
    cancel.addEventListener('click', back);
    setTimeout(() => input.focus(), 0);
    // Un modulo: Invio salva, Esc annulla.
    const form = el('form', { className: 'max-editor' }, input, save, useDefault, cancel,
      el('span', { className: 'hint', text: '0 = nessun limite' }));
    form.addEventListener('submit', (event) => {
      event.preventDefault();
      const text = input.value.trim();
      if (text === '') {
        submit(save, null);
        return;
      }
      const value = Number(text);
      if (!Number.isInteger(value) || value < 0) {
        setError('Il massimo è un numero intero da 0 in su (0 = nessun limite).');
        return;
      }
      submit(save, value);
    });
    form.addEventListener('keydown', (event) => {
      if (event.key === 'Escape') back();
    });
    return form;
  }

  async function renderDevices(user, box) {
    const waiting = el('p', { className: 'hint', text: 'Carico i dispositivi…' });
    box.append(waiting);
    try {
      const data = await api('GET', '/users/' + encodeURIComponent(user.id) + '/devices');
      waiting.remove();
      if (data.devices.length === 0) {
        box.append(el('p', { className: 'hint', text: 'Nessun dispositivo attivo.' }));
        return;
      }
      const list = el('ul', { className: 'devices', 'aria-label': 'Dispositivi di ' + user.username });
      for (const device of data.devices) {
        const revoke = guardedButton('Revoca', 'Sì, revoca', 'danger');
        revoke.addEventListener('click', () => {
          if (!secondClick(revoke, 'Revoca il dispositivo «' + device.name + '»: dovrà accedere di nuovo.')) return;
          run(revoke, async () => {
            await api('DELETE', '/users/' + encodeURIComponent(user.id) + '/devices/' + encodeURIComponent(device.id));
            await reloadUsers(user);
            setStatus('Dispositivo «' + device.name + '» revocato.');
          });
        });
        list.append(el('li', {},
          el('span', {},
            el('strong', { text: device.name }),
            el('span', {
              className: 'sub',
              text: 'Creato ' + formatDate(device.created_at) + ' · ultimo accesso ' + formatDate(device.last_seen_at),
            })),
          revoke));
      }
      box.append(list);
    } catch (e) {
      if (!(e instanceof Unauthorized)) {
        waiting.remove();
        box.append(el('p', { className: 'note note-danger', text: e.message }));
      }
    }
  }

  // --- Modelli (GET /admin/api/models, POST .../install, DELETE ...) ----------------------

  let modelsTimer = null;

  function formatBytes(bytes) {
    // Zero: un modello che non sta su questo disco (un servizio di embedding).
    if (typeof bytes !== 'number' || !Number.isFinite(bytes) || bytes === 0) return '—';
    if (bytes >= 1e9) return (bytes / 1e9).toLocaleString('it-IT', { maximumFractionDigits: 1 }) + ' GB';
    return Math.max(1, Math.round(bytes / 1e6)).toLocaleString('it-IT') + ' MB';
  }

  function formatMemory(mb) {
    if (typeof mb !== 'number' || !Number.isFinite(mb)) return '—';
    if (mb >= 1024) return (mb / 1024).toLocaleString('it-IT', { maximumFractionDigits: 1 }) + ' GB';
    return mb.toLocaleString('it-IT') + ' MB';
  }

  const modelStates = {
    ready: { text: 'Pronto', className: 'pill-ok' },
    downloading: { text: 'Scaricamento', className: 'pill-warn' },
    loading: { text: 'In caricamento', className: 'pill-warn' },
    missing: { text: 'Non installato', className: '' },
  };

  const modelKinds = { embedding: 'Ricerca', speech: 'Trascrizione', llm: 'Assistente' };

  function stateCell(model) {
    const state = modelStates[model.state] || { text: model.state, className: '' };
    const box = el('div', { className: 'model-state' }, el('span', { className: 'pill ' + state.className, text: state.text }));
    if (model.state === 'downloading') {
      const percent = typeof model.percent === 'number' ? model.percent : 0;
      box.append(el('progress', { max: '100', value: String(percent), 'aria-label': 'Avanzamento di ' + model.id }),
        el('span', { className: 'hint', text: percent + '%' }));
    }
    const cell = el('td', {}, box);
    if (model.state === 'missing' && model.error) {
      cell.append(el('p', { className: 'note note-danger', text: model.error }));
    }
    return cell;
  }

  // Prima di «Installa»: quanto serve e quanto c'è ora, e il motivo se non basta.
  function modelActions(model, host) {
    const box = el('div', { className: 'model-actions' });
    const details = model.llm || null;
    if (model.state === 'downloading') {
      box.append(el('span', { className: 'hint', text: 'Scaricamento in corso…' }));
      return box;
    }
    // Il servizio scelto in «Embedding» non è un file di questo server: non c'è nulla da installare.
    if (model.kind === 'embedding' && !model.total_bytes) {
      box.append(el('span', { className: 'hint', text: 'Servizio interno: si cambia dalla sezione «Embedding».' }));
      return box;
    }
    // In caricamento è già installato.
    if (model.state === 'loading') return box;
    if (model.state === 'ready') {
      if (details === null) return box;
      const remove = guardedButton('Rimuovi', 'Sì, rimuovi', 'danger');
      remove.addEventListener('click', () => {
        if (!secondClick(remove, 'Rimuove il modello «' + model.id + '» da questo server.')) return;
        runModels(remove, async () => {
          await api('DELETE', '/models/' + encodeURIComponent(model.id));
          setModelsStatus('Modello «' + model.id + '» rimosso.');
          await loadModels();
        });
      });
      box.append(remove);
      return box;
    }
    let blocked = null;
    if (details !== null) {
      const memoryNow = host && typeof host.available_memory_mb === 'number' ? host.available_memory_mb : null;
      const diskNow = host && typeof host.available_disk_bytes === 'number' ? host.available_disk_bytes : null;
      if (typeof details.license_url === 'string' && details.license_url !== '') {
        box.append(el('span', { className: 'hint', text: 'Licenza: ' + details.license + '. Condizioni: ' + details.license_url + '. Installando il modello si accettano queste condizioni.' }));
      }
      if (memoryNow !== null && memoryNow < details.ram_required_mb) {
        box.append(el('span', { className: 'note note-warn', text: 'La memoria libera ora è ' + formatMemory(memoryNow) + ': potrebbe non bastare.' }));
      }
      if (diskNow !== null && diskNow < details.disk_required_bytes) {
        blocked = 'Spazio su disco insufficiente: servono ' + formatBytes(details.disk_required_bytes) + '.';
      } else if (details.pinned === false) {
        blocked = 'Modello non ancora disponibile.';
      }
    }
    const install = el('button', { type: 'button', className: 'secondary', text: 'Installa', disabled: blocked !== null });
    install.addEventListener('click', () => runModels(install, async () => {
      await api('POST', '/models/' + encodeURIComponent(model.id) + '/install');
      setModelsStatus('Installazione di «' + model.id + '» avviata.');
      await loadModels();
    }));
    box.append(install);
    if (blocked !== null) box.append(el('span', { className: 'note note-danger', text: blocked }));
    return box;
  }

  // Vero se ha disegnato la tabella.
  function renderModels(models, host) {
    const body = byId('models-body');
    // Un «Rimuovi» armato aspetta il secondo clic: la tabella si ridisegna al giro dopo.
    if (body.querySelector('button.confirm')) return false;
    // Durante uno scaricamento la tabella si ridisegna ogni due secondi: il fuoco resta sulla sua riga.
    const row = document.activeElement.closest('tr[data-model]');
    const focused = row ? row.getAttribute('data-model') : null;
    clear(body);
    for (const model of models) {
      const details = model.llm || null;
      body.append(el('tr', { 'data-model': model.id },
        el('td', {},
          el('strong', { className: 'mono', text: model.id }),
          el('span', { className: 'sub', text: modelKinds[model.kind] || model.kind })),
        stateCell(model),
        el('td', { text: details ? formatBytes(details.disk_required_bytes) : formatBytes(model.total_bytes), className: 'num', 'data-label': 'Disco' }),
        el('td', { text: details ? formatMemory(details.ram_required_mb) : '—', className: 'num', 'data-label': 'Memoria' }),
        el('td', { text: details ? details.license : '—', className: 'nowrap', 'data-label': 'Licenza' }),
        el('td', { className: 'col-actions', 'data-wide': true }, modelActions(model, host))));
      if (model.id === focused) {
        const button = body.lastChild.querySelector('button');
        if (button) button.focus();
      }
    }
    return true;
  }

  function setModelsStatus(message) {
    byId('models-status').textContent = message || '';
  }

  function setModelsError(message) {
    note('models-error', message);
  }

  async function loadModels() {
    clearTimeout(modelsTimer);
    modelsTimer = null;
    let data;
    try {
      data = await api('GET', '/models');
    } catch (e) {
      if (e instanceof Unauthorized) throw e;
      // Senza le rotte di calcolo la sezione non c'è; un altro errore si legge nella sezione.
      byId('models-section').hidden = e.status === 404;
      if (e.status !== 404) setModelsError(e.message);
      syncNav();
      return;
    }
    setModelsError('');
    lastModels = data.models || [];
    byId('models-section').hidden = false;
    const host = data.host || {};
    const parts = [];
    if (typeof host.available_memory_mb === 'number') parts.push('Memoria libera: ' + formatMemory(host.available_memory_mb));
    if (typeof host.available_disk_bytes === 'number') parts.push('Spazio libero: ' + formatBytes(host.available_disk_bytes));
    byId('models-host').textContent = parts.join(' · ');
    const drawn = renderModels(data.models, host);
    // Anche quando il disegno è saltato: uno scaricamento finito intanto deve comparire.
    if (!drawn || data.models.some((m) => m.state === 'downloading')) {
      modelsTimer = setTimeout(() => loadModels().catch(() => {}), 2000);
    }
  }

  const runModels = runner(setModelsError, () => setModelsStatus(''));

  // --- Assistente: chi risponde (spento, API compatibile OpenAI, modello locale) -------------
  // La chiave del provider non torna mai dal server: solo da dove viene e le ultime lettere.

  let lastModels = [];

  function setLlmStatus(message) {
    byId('llm-status').textContent = message || '';
  }

  function setLlmError(message) {
    note('llm-error', message);
  }

  function llmProvider() {
    const checked = document.querySelector('input[name="llm-provider"]:checked');
    return checked ? checked.value : 'none';
  }

  function showLlmFields() {
    const provider = llmProvider();
    byId('llm-openai').hidden = provider !== 'openai-compatible';
    byId('llm-key-box').hidden = provider !== 'openai-compatible';
    byId('llm-builtin').hidden = provider !== 'builtin';
  }

  function describeLlm(data) {
    const parts = [];
    if (data.active) {
      parts.push(data.active.provider === 'builtin'
        ? 'Modello locale ' + data.active.model + '.'
        : 'Modello ' + data.active.model + ' tramite ' + data.active.label + '.');
    } else {
      parts.push('Nessun assistente.');
    }
    parts.push(data.source === 'admin' ? 'Impostazioni salvate da questo pannello.' : 'Impostazioni della configurazione del server.');
    if (data.restart_required) parts.push('Il modello locale scelto parte al prossimo riavvio del server.');
    return parts.join(' ');
  }

  function renderLlm(data) {
    setLlmError('');
    note('llm-load-error', '');
    byId('llm-form').hidden = false;
    byId('llm-section').hidden = false;
    byId('llm-now').textContent = describeLlm(data);
    for (const radio of document.querySelectorAll('input[name="llm-provider"]')) {
      radio.checked = radio.value === data.provider;
    }
    byId('llm-base-url').value = data.base_url || '';
    byId('llm-model').value = data.model || '';
    byId('llm-allowed').value = (data.allowed_models || []).join(', ');
    byId('llm-cap').value = String(data.max_tokens_per_day_per_user || 0);
    byId('llm-key').value = '';
    byId('llm-key-remove').checked = false;
    const key = data.api_key;
    const hint = byId('llm-key-hint');
    if (!key) hint.textContent = 'Nessuna chiave impostata: un servizio esterno come OpenAI la chiede.';
    else if (key.from === 'admin') hint.textContent = 'Chiave impostata da questo pannello (' + key.hint + '). Lascia il campo vuoto per tenerla.';
    else hint.textContent = 'Chiave della configurazione del server (' + key.hint + '). Lascia il campo vuoto per usarla.';
    byId('llm-key-remove-label').hidden = !(key && key.from === 'admin');
    const select = byId('llm-builtin-model');
    clear(select);
    const local = lastModels.filter((model) => model.kind === 'llm');
    for (const model of local) {
      const ready = model.state === 'ready';
      select.append(el('option', { value: model.id, text: model.id + (ready ? '' : ' (non scaricato)') }));
    }
    if (data.builtin_model && !local.some((model) => model.id === data.builtin_model)) {
      select.append(el('option', { value: data.builtin_model, text: data.builtin_model }));
    }
    if (data.builtin_model) select.value = data.builtin_model;
    showLlmFields();
  }

  async function loadLlm() {
    let data;
    try {
      data = await api('GET', '/llm');
    } catch (e) {
      if (e instanceof Unauthorized) throw e;
      // Senza le rotte di calcolo la sezione non c'è; un altro errore si legge nella sezione,
      // senza il modulo: vuoto, salverebbe «nessun assistente».
      byId('llm-section').hidden = e.status === 404;
      byId('llm-form').hidden = true;
      byId('llm-now').textContent = '—';
      note('llm-load-error', e.status === 404 ? '' : e.message);
      syncNav();
      return;
    }
    renderLlm(data);
  }

  async function saveLlm() {
    const capText = byId('llm-cap').value.trim();
    const cap = capText ? Number(capText) : 0;
    if (!Number.isInteger(cap) || cap < 0) {
      setLlmError('Il limite è un numero intero da 0 in su (0: nessun limite).');
      return;
    }
    const body = {
      provider: llmProvider(),
      base_url: byId('llm-base-url').value.trim() || null,
      model: byId('llm-model').value.trim() || null,
      allowed_models: byId('llm-allowed').value.split(',').map((name) => name.trim()).filter(Boolean),
      max_tokens_per_day_per_user: cap,
      builtin_model: byId('llm-builtin-model').value || null,
      remove_api_key: byId('llm-key-remove').checked,
    };
    // La chiave scritta resta nel campo finché il salvataggio non riesce (poi lo svuota `renderLlm`).
    const key = byId('llm-key').value.trim();
    if (key) body.api_key = key;
    const data = await api('PUT', '/llm', body);
    renderLlm(data);
    setLlmStatus(data.restart_required ? 'Salvate: il modello locale parte al prossimo riavvio del server.' : 'Salvate e applicate.');
  }

  async function resetLlm() {
    const data = await api('DELETE', '/llm');
    renderLlm(data);
    setLlmStatus('Vale di nuovo la configurazione del server.');
  }

  const runLlm = runner(setLlmError, () => setLlmStatus(''));

  // --- Modello di embedding: chi calcola i vettori ------------------------------------------
  // Il modello locale o un'API interna, sempre con i vettori della misura dell'indice: il server
  // prova il modello prima di salvare. Cambiarlo chiede una conferma (le app reindicizzano).

  let lastEmbedding = null;

  function setEmbeddingStatus(message) {
    byId('embedding-status').textContent = message || '';
  }

  function setEmbeddingError(message) {
    note('embedding-error', message);
    if (message) setEmbeddingStatus('');
  }

  function embeddingProvider() {
    const checked = document.querySelector('input[name="embedding-provider"]:checked');
    return checked ? checked.value : 'builtin';
  }

  // Il modello che varrebbe salvando ora; vuoto finché non è scritto.
  function embeddingChoice() {
    if (!lastEmbedding) return '';
    return embeddingProvider() === 'builtin' ? lastEmbedding.builtin.model : byId('embedding-model').value.trim();
  }

  // Vero se salvando ora il modello può cambiare: le app dovrebbero reindicizzare.
  function embeddingChanges() {
    const choice = embeddingChoice();
    if (choice === '') return false;
    if (choice !== lastEmbedding.active.model.split('#')[0]) return true;
    // Lo stesso nome a un altro indirizzo può dare altri vettori: allora il server lo rinomina.
    return embeddingProvider() === 'openai-compatible' && byId('embedding-base-url').value.trim() !== (lastEmbedding.base_url || '');
  }

  function showEmbeddingFields() {
    const remote = embeddingProvider() === 'openai-compatible';
    byId('embedding-openai').hidden = !remote;
    byId('embedding-key-box').hidden = !remote;
    byId('embedding-warning').hidden = !embeddingChanges();
    disarm(byId('embedding-save'));
  }

  function renderEmbedding(data) {
    lastEmbedding = data;
    setEmbeddingError('');
    note('embedding-load-error', '');
    byId('embedding-form').hidden = false;
    byId('embedding-section').hidden = false;
    const failure = data.last_error;
    note('embedding-last-error', failure ? 'Ultimo errore del servizio (' + formatDate(failure.at) + '): ' + failure.message : '');
    byId('embedding-dim').textContent = String(data.dimensions);
    const local = data.active.model === data.builtin.model;
    byId('embedding-now').textContent = data.active.model + ' · ' + data.active.dimensions + ' valori per vettore · ' +
      (local ? 'modello locale di questo server' : 'tramite ' + (data.base_url || 'un servizio interno'));
    for (const radio of document.querySelectorAll('input[name="embedding-provider"]')) {
      radio.checked = radio.value === data.provider;
    }
    byId('embedding-base-url').value = data.base_url || '';
    byId('embedding-model').value = data.model || '';
    byId('embedding-key').value = '';
    byId('embedding-key-remove').checked = false;
    const key = data.api_key;
    byId('embedding-key-hint').textContent = key
      ? 'Chiave salvata (' + key.hint + '). Lascia il campo vuoto per tenerla.'
      : 'Nessuna chiave salvata: molti servizi interni non la chiedono.';
    byId('embedding-key-remove-label').hidden = !key;
    byId('embedding-builtin-note').hidden = data.builtin.state === 'ready' || data.builtin.state === 'loading';
    showEmbeddingFields();
  }

  async function loadEmbedding() {
    let data;
    try {
      data = await api('GET', '/embedding');
    } catch (e) {
      if (e instanceof Unauthorized) throw e;
      // Senza le rotte di calcolo la sezione non c'è; un altro errore si legge nella sezione, senza il modulo.
      byId('embedding-section').hidden = e.status === 404;
      byId('embedding-form').hidden = true;
      byId('embedding-now').textContent = '—';
      note('embedding-load-error', e.status === 404 ? '' : e.message);
      syncNav();
      return;
    }
    renderEmbedding(data);
  }

  async function saveEmbedding() {
    const provider = embeddingProvider();
    const body = { provider, base_url: null, model: null, remove_api_key: false };
    if (provider === 'openai-compatible') {
      body.base_url = byId('embedding-base-url').value.trim() || null;
      body.model = byId('embedding-model').value.trim() || null;
      body.remove_api_key = byId('embedding-key-remove').checked;
      if (!body.base_url || !body.model) {
        setEmbeddingError('Scrivi indirizzo base e nome del modello.');
        return;
      }
    }
    // La chiave scritta resta nel campo finché il salvataggio non riesce (poi lo svuota `renderEmbedding`).
    const key = byId('embedding-key').value.trim();
    if (key) body.api_key = key;
    setEmbeddingStatus('Provo il modello…');
    const data = await api('PUT', '/embedding', body);
    renderEmbedding(data);
    await loadModels();
    setEmbeddingStatus('Salvato e applicato: ' + data.active.model + ', ' + data.active.dimensions + ' valori per vettore.');
  }

  const runEmbedding = runner(setEmbeddingError, () => setEmbeddingStatus(''));

  function toggleNewUser(show) {
    const form = byId('new-user-form');
    form.hidden = !show;
    byId('new-user-toggle').setAttribute('aria-expanded', show ? 'true' : 'false');
    if (show) {
      form.reset();
      byId('new-username').focus();
    }
  }

  async function copySecret() {
    const value = byId('secret-value').textContent;
    const button = byId('secret-copy');
    try {
      await navigator.clipboard.writeText(value);
      button.textContent = 'Copiata';
    } catch (e) {
      // Senza appunti: seleziona il testo, così lo si copia a mano.
      const range = document.createRange();
      range.selectNodeContents(byId('secret-value'));
      const selection = window.getSelection();
      selection.removeAllRanges();
      selection.addRange(range);
      button.textContent = 'Selezionata: copia con Ctrl+C';
    }
  }

  function init() {
    byId('key-form').addEventListener('submit', (event) => {
      event.preventDefault();
      const value = byId('key-input').value.trim();
      byId('key-input').value = '';
      if (!value) return;
      adminKey = value;
      const started = session;
      runKey(byId('key-submit'), loadAll).then(() => {
        // Non entrati (il server non risponde, un errore): la chiave non resta in memoria. Se
        // intanto si è usciti e rientrati, la chiave è quella del nuovo accesso: resta.
        if (session === started && byId('shell').hidden) adminKey = null;
      });
    });
    document.addEventListener('keydown', (event) => {
      if (event.key === 'Escape') disarmAll();
      // Invio tenuto premuto ripete il clic: su un pulsante conta solo il primo.
      if (event.repeat && event.key === 'Enter' && event.target instanceof Element && event.target.closest('button')) event.preventDefault();
    });
    // Dove un clic non dà il fuoco al pulsante (Safari) l'uscita dal pulsante non si vede: un clic
    // su qualunque altra cosa disarma, prima che quel clic faccia il suo.
    document.addEventListener('click', (event) => disarmAll(event.target.closest('button')), true);
    byId('lock').addEventListener('click', () => lock(''));
    for (const item of document.querySelectorAll('.nav-item[data-view]')) {
      item.addEventListener('click', () => showView(item.getAttribute('data-view')));
    }
    byId('refresh').addEventListener('click', (event) => run(event.currentTarget, loadUsers));
    byId('new-user-toggle').addEventListener('click', () => toggleNewUser(byId('new-user-form').hidden));
    byId('new-user-cancel').addEventListener('click', () => toggleNewUser(false));
    byId('new-user-form').addEventListener('submit', (event) => {
      event.preventDefault();
      const username = byId('new-username').value.trim();
      const email = byId('new-email').value.trim();
      const maxText = byId('new-max').value.trim();
      const body = { username };
      if (email) body.email = email;
      if (maxText) {
        const value = Number(maxText);
        if (!Number.isInteger(value) || value < 0) {
          setError('Il massimo è un numero intero da 0 in su (0 = nessun limite).');
          return;
        }
        body.max_devices = value;
      }
      run(byId('new-user-submit'), async () => {
        const created = await api('POST', '/users', body);
        toggleNewUser(false);
        showSecret(
          'Utente «' + created.username + '» creato. Password provvisoria, valida fino a ' +
            formatDate(created.temporary_expires_at) + ':',
          created.temporary_password);
        await loadUsers();
      });
    });
    byId('secret-copy').addEventListener('click', copySecret);
    byId('secret-close').addEventListener('click', hideSecret);
    byId('models-refresh').addEventListener('click', (event) => runModels(event.currentTarget, loadModels));
    for (const radio of document.querySelectorAll('input[name="llm-provider"]')) {
      radio.addEventListener('change', showLlmFields);
    }
    byId('llm-form').addEventListener('submit', (event) => {
      event.preventDefault();
      runLlm(byId('llm-save'), saveLlm);
    });
    byId('llm-reset').addEventListener('click', (event) => {
      const button = event.currentTarget;
      if (!secondClick(button, 'Scarta le impostazioni salvate da questo pannello, chiave compresa: vale di nuovo la configurazione del server.')) return;
      runLlm(button, resetLlm);
    });
    byId('llm-refresh').addEventListener('click', (event) => runLlm(event.currentTarget, async () => {
      await loadModels();
      await loadLlm();
    }));
    for (const radio of document.querySelectorAll('input[name="embedding-provider"]')) {
      radio.addEventListener('change', showEmbeddingFields);
    }
    byId('embedding-model').addEventListener('input', showEmbeddingFields);
    byId('embedding-base-url').addEventListener('input', showEmbeddingFields);
    byId('embedding-form').addEventListener('submit', (event) => {
      event.preventDefault();
      const save = byId('embedding-save');
      // Un altro modello: tutte le app reindicizzano, quindi un secondo clic.
      if (embeddingChanges() && !secondClick(save, 'Cambia il modello di embedding: se i vettori cambiano, ogni app chiederà di reindicizzare.')) return;
      runEmbedding(save, saveEmbedding);
    });
    byId('embedding-refresh').addEventListener('click', (event) => runEmbedding(event.currentTarget, loadEmbedding));
    lock('');
  }

  document.addEventListener('DOMContentLoaded', init);
})();
