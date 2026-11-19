// Pagina /admin di qamille-server.
//
// Regole: la chiave amministratore sta solo in una variabile di questa pagina e va
// in Authorization; nessun cookie, nessun salvataggio nel browser. I nodi si creano
// con createElement e textContent, mai interpretando testo come HTML. Un 401
// svuota la chiave e la richiede.
(() => {
  'use strict';

  let adminKey = null;
  let defaultMax = 0;
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
      else if (name.startsWith('on')) node.addEventListener(name.slice(2), value);
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
    let response;
    try {
      response = await fetch('/admin/api' + path, init);
    } catch (e) {
      throw new Error('Il server non risponde.');
    }
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
    if (!response.ok) {
      const message = data && data.error && data.error.message;
      const failure = new Error(message || 'Errore ' + response.status + '.');
      failure.status = response.status;
      throw failure;
    }
    return data;
  }

  function setStatus(message) {
    byId('status').textContent = message || '';
  }

  function setError(message) {
    const node = byId('error');
    node.textContent = message || '';
    node.hidden = !message;
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
    openDetails.clear();
    hideSecret();
    clear(byId('users-body'));
    byId('users-section').hidden = true;
    byId('models-section').hidden = true;
    clearTimeout(modelsTimer);
    modelsTimer = null;
    removeArmed = null;
    clear(byId('models-body'));
    setModelsStatus('');
    setModelsError('');
    byId('lock').hidden = true;
    byId('key-section').hidden = false;
    byId('key-input').value = '';
    const error = byId('key-error');
    error.textContent = message || '';
    error.hidden = !message;
    setStatus('');
    setError('');
    byId('key-input').focus();
  }

  // Esegue un'azione con il pulsante disattivato; gli errori vanno nel riquadro.
  async function run(button, action) {
    if (button) button.disabled = true;
    setError('');
    try {
      await action();
    } catch (e) {
      if (!(e instanceof Unauthorized)) setError(e.message);
    } finally {
      if (button) button.disabled = false;
    }
  }

  function stateOf(user) {
    if (user.disabled_at) return { text: 'Disattivato', className: 'state-disabled' };
    if (user.password_temporary) {
      const expired = user.temporary_expires_at && new Date(user.temporary_expires_at) <= new Date();
      return {
        text: expired ? 'Provvisoria scaduta' : 'Password provvisoria',
        className: 'state-temporary',
      };
    }
    return { text: 'Attivo', className: '' };
  }

  function llmTokensCell(user) {
    const usage = user.usage || {};
    const incoming = usage.llm_prompt_tokens || 0;
    const outgoing = usage.llm_completion_tokens || 0;
    return el('td', {
      text: (incoming + outgoing).toLocaleString('it-IT'),
      className: 'num',
      title: incoming.toLocaleString('it-IT') + ' in entrata / ' + outgoing.toLocaleString('it-IT') + ' in uscita',
    });
  }

  function maxText(user) {
    const value = user.max_devices === null || user.max_devices === undefined ? defaultMax : user.max_devices;
    const label = value === 0 ? 'illimitato' : String(value);
    return user.max_devices === null || user.max_devices === undefined ? label + ' (predefinito)' : label;
  }

  async function loadUsers() {
    const data = await api('GET', '/users');
    defaultMax = data.default_max_devices;
    llmOn = Boolean(data.llm);
    byId('llm-col').hidden = !data.llm;
    renderUsers(data.users);
    byId('key-section').hidden = true;
    byId('users-section').hidden = false;
    byId('lock').hidden = false;
    await loadModels();
  }

  function renderUsers(users) {
    const body = byId('users-body');
    clear(body);
    byId('empty').hidden = users.length > 0;
    byId('users-table').hidden = users.length === 0;
    for (const user of users) {
      const state = stateOf(user);
      const row = el('tr', { 'data-user': user.username },
        el('td', { text: user.username }),
        el('td', { text: user.email || '—' }),
        el('td', { text: state.text, className: state.className }),
        el('td', { text: String(user.active_devices), className: 'num' }),
        el('td', { text: maxText(user), className: 'num' }),
        llmOn ? llmTokensCell(user) : null,
        el('td', {}, rowActions(user)));
      body.append(row);
      if (openDetails.has(user.id)) {
        const details = el('tr', { className: 'details' });
        body.append(details);
        renderDevices(user, details);
      }
    }
  }

  function rowActions(user) {
    const box = el('div', { className: 'row-actions' });

    const devices = el('button', {
      type: 'button',
      className: 'secondary',
      text: openDetails.has(user.id) ? 'Nascondi dispositivi' : 'Dispositivi',
      'aria-expanded': openDetails.has(user.id) ? 'true' : 'false',
    });
    devices.addEventListener('click', () => run(devices, async () => {
      if (openDetails.has(user.id)) openDetails.delete(user.id);
      else openDetails.add(user.id);
      await loadUsers();
    }));

    const reset = el('button', { type: 'button', className: 'secondary', text: 'Reimposta password' });
    reset.addEventListener('click', () => run(reset, async () => {
      const result = await api('POST', '/users/' + encodeURIComponent(user.id) + '/reset-password');
      await loadUsers();
      showSecret(
        'Nuova password provvisoria per «' + user.username + '», valida fino a ' +
          formatDate(result.temporary_expires_at) + ' (i dispositivi sono stati disconnessi):',
        result.temporary_password);
    }));

    const toggle = el('button', {
      type: 'button',
      className: 'secondary',
      text: user.disabled_at ? 'Riattiva' : 'Disattiva',
    });
    toggle.addEventListener('click', () => run(toggle, async () => {
      const action = user.disabled_at ? 'enable' : 'disable';
      await api('POST', '/users/' + encodeURIComponent(user.id) + '/' + action);
      await loadUsers();
      setStatus(user.disabled_at ? '«' + user.username + '» riattivato.' : '«' + user.username + '» disattivato.');
    }));

    const max = el('button', { type: 'button', className: 'secondary', text: 'Massimo…' });
    max.addEventListener('click', () => {
      box.replaceChildren(maxEditor(user));
    });

    const remove = el('button', { type: 'button', className: 'danger', text: 'Cancella' });
    let armed = null;
    remove.addEventListener('click', () => {
      if (!armed) {
        remove.textContent = 'Conferma: cancella «' + user.username + '»';
        remove.classList.add('confirm');
        armed = setTimeout(() => {
          armed = null;
          remove.textContent = 'Cancella';
          remove.classList.remove('confirm');
        }, 6000);
        return;
      }
      clearTimeout(armed);
      armed = null;
      run(remove, async () => {
        await api('DELETE', '/users/' + encodeURIComponent(user.id));
        openDetails.delete(user.id);
        await loadUsers();
        setStatus('«' + user.username + '» cancellato con i suoi dispositivi.');
      });
    });

    box.append(devices, reset, toggle, max, remove);
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
    const save = el('button', { type: 'button', text: 'Salva' });
    const useDefault = el('button', { type: 'button', className: 'secondary', text: 'Predefinito' });
    const cancel = el('button', { type: 'button', className: 'secondary', text: 'Annulla' });
    const submit = (button, value) => run(button, async () => {
      await api('POST', '/users/' + encodeURIComponent(user.id) + '/max-devices', { max_devices: value });
      await loadUsers();
      setStatus('Massimo di dispositivi aggiornato per «' + user.username + '».');
    });
    save.addEventListener('click', () => {
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
    useDefault.addEventListener('click', () => submit(useDefault, null));
    cancel.addEventListener('click', () => run(cancel, loadUsers));
    setTimeout(() => input.focus(), 0);
    return el('div', { className: 'max-editor' }, input, save, useDefault, cancel,
      el('span', { className: 'hint', text: '0 = nessun limite' }));
  }

  async function renderDevices(user, row) {
    const cell = el('td', { colspan: llmOn ? '7' : '6' }, el('span', { className: 'hint', text: 'Carico i dispositivi…' }));
    row.append(cell);
    try {
      const data = await api('GET', '/users/' + encodeURIComponent(user.id) + '/devices');
      clear(cell);
      if (data.devices.length === 0) {
        cell.append(el('span', { className: 'hint', text: 'Nessun dispositivo attivo.' }));
        return;
      }
      const list = el('ul', { className: 'devices', 'aria-label': 'Dispositivi di ' + user.username });
      for (const device of data.devices) {
        const revoke = el('button', { type: 'button', className: 'danger', text: 'Revoca' });
        revoke.addEventListener('click', () => run(revoke, async () => {
          await api('DELETE', '/users/' + encodeURIComponent(user.id) + '/devices/' + encodeURIComponent(device.id));
          await loadUsers();
          setStatus('Dispositivo «' + device.name + '» revocato.');
        }));
        list.append(el('li', { 'data-device': device.name },
          el('span', {},
            el('strong', { text: device.name }),
            el('span', {
              className: 'hint',
              text: ' · creato ' + formatDate(device.created_at) + ' · ultimo accesso ' + formatDate(device.last_seen_at),
            })),
          revoke));
      }
      cell.append(list);
    } catch (e) {
      if (!(e instanceof Unauthorized)) {
        clear(cell);
        cell.append(el('span', { className: 'error', text: e.message }));
      }
    }
  }

  // --- Modelli (GET /admin/api/models, POST .../install, DELETE ...) ----------------------

  let modelsTimer = null;
  let removeArmed = null;

  function formatBytes(bytes) {
    if (typeof bytes !== 'number' || !Number.isFinite(bytes)) return '—';
    if (bytes >= 1e9) return (bytes / 1e9).toLocaleString('it-IT', { maximumFractionDigits: 1 }) + ' GB';
    return Math.max(1, Math.round(bytes / 1e6)).toLocaleString('it-IT') + ' MB';
  }

  function formatMemory(mb) {
    if (typeof mb !== 'number' || !Number.isFinite(mb)) return '—';
    if (mb >= 1024) return (mb / 1024).toLocaleString('it-IT', { maximumFractionDigits: 1 }) + ' GB';
    return mb.toLocaleString('it-IT') + ' MB';
  }

  const modelStates = {
    ready: 'Pronto',
    downloading: 'Scaricamento',
    loading: 'In caricamento',
    missing: 'Non installato',
  };

  function stateCell(model) {
    const cell = el('td', {}, el('span', { text: modelStates[model.state] || model.state }));
    if (model.state === 'downloading') {
      const percent = typeof model.percent === 'number' ? model.percent : 0;
      cell.append(' ', el('progress', { max: '100', value: String(percent), 'aria-label': 'Avanzamento di ' + model.id }),
        el('span', { className: 'hint', text: ' ' + percent + '%' }));
    }
    if (model.state === 'missing' && model.error) {
      cell.append(el('br'), el('span', { className: 'error', text: model.error }));
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
    if (model.state === 'ready') {
      if (details === null) return box;
      const remove = el('button', { type: 'button', className: 'danger', text: 'Rimuovi' });
      remove.addEventListener('click', () => {
        if (removeArmed !== model.id) {
          removeArmed = model.id;
          remove.textContent = 'Sicuro? Rimuovi';
          return;
        }
        removeArmed = null;
        run(remove, async () => {
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
      const lines = ['Servono ' + formatMemory(details.ram_required_mb) + ' di memoria e ' + formatBytes(details.disk_required_bytes) + ' di disco.'];
      if (memoryNow !== null) lines.push('Memoria libera ora: ' + formatMemory(memoryNow) + '.');
      if (diskNow !== null) lines.push('Spazio libero: ' + formatBytes(diskNow) + '.');
      box.append(el('span', { className: 'hint', text: lines.join(' ') }));
      if (typeof details.license_url === 'string' && details.license_url !== '') {
        box.append(el('br'), el('span', { className: 'hint', text: 'Licenza: ' + details.license + '. Condizioni: ' + details.license_url + '. Installando il modello si accettano queste condizioni.' }));
      }
      if (memoryNow !== null && memoryNow < details.ram_required_mb) {
        box.append(el('br'), el('span', { className: 'error', text: 'La memoria libera ora è ' + formatMemory(memoryNow) + ': potrebbe non bastare.' }));
      }
      if (diskNow !== null && diskNow < details.disk_required_bytes) {
        blocked = 'Spazio su disco insufficiente: servono ' + formatBytes(details.disk_required_bytes) + '.';
      } else if (details.pinned === false) {
        blocked = 'Modello non ancora disponibile.';
      }
    }
    const install = el('button', { type: 'button', text: 'Installa', disabled: blocked !== null, title: blocked });
    install.addEventListener('click', () => run(install, async () => {
      await api('POST', '/models/' + encodeURIComponent(model.id) + '/install');
      setModelsStatus('Installazione di «' + model.id + '» avviata.');
      await loadModels();
    }));
    box.append(el('br'), install);
    if (blocked !== null) box.append(el('br'), el('span', { className: 'error', text: blocked }));
    return box;
  }

  function renderModels(models, host) {
    const body = byId('models-body');
    clear(body);
    for (const model of models) {
      const details = model.llm || null;
      body.append(el('tr', { 'data-model': model.id },
        el('td', { text: model.id }),
        el('td', { text: model.kind }),
        stateCell(model),
        el('td', { text: details ? formatBytes(details.disk_required_bytes) : formatBytes(model.total_bytes), className: 'num' }),
        el('td', { text: details ? formatMemory(details.ram_required_mb) : '—', className: 'num' }),
        el('td', { text: details ? details.license : '—' }),
        el('td', {}, modelActions(model, host))));
    }
  }

  function setModelsStatus(message) {
    byId('models-status').textContent = message || '';
  }

  function setModelsError(message) {
    const node = byId('models-error');
    node.textContent = message || '';
    node.hidden = !message;
  }

  async function loadModels() {
    clearTimeout(modelsTimer);
    modelsTimer = null;
    let data;
    try {
      data = await api('GET', '/models');
    } catch (e) {
      if (e instanceof Unauthorized) throw e;
      // Senza le rotte di calcolo la sezione non c'è.
      if (e.status === 404) byId('models-section').hidden = true;
      else setModelsError(e.message);
      return;
    }
    setModelsError('');
    byId('models-section').hidden = false;
    const host = data.host || {};
    const parts = [];
    if (typeof host.available_memory_mb === 'number') parts.push('Memoria libera: ' + formatMemory(host.available_memory_mb));
    if (typeof host.available_disk_bytes === 'number') parts.push('Spazio libero: ' + formatBytes(host.available_disk_bytes));
    byId('models-host').textContent = parts.join(' · ');
    renderModels(data.models, host);
    if (data.models.some((m) => m.state === 'downloading')) {
      modelsTimer = setTimeout(() => loadModels().catch(() => {}), 2000);
    }
  }

  function toggleNewUser(show) {
    const form = byId('new-user-form');
    form.hidden = !show;
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
      byId('key-error').hidden = true;
      run(byId('key-submit'), loadUsers);
    });
    byId('lock').addEventListener('click', () => lock(''));
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
        await loadUsers();
        showSecret(
          'Utente «' + created.username + '» creato. Password provvisoria, valida fino a ' +
            formatDate(created.temporary_expires_at) + ':',
          created.temporary_password);
      });
    });
    byId('secret-copy').addEventListener('click', copySecret);
    byId('secret-close').addEventListener('click', hideSecret);
    byId('models-refresh').addEventListener('click', (event) => run(event.currentTarget, loadModels));
    lock('');
  }

  document.addEventListener('DOMContentLoaded', init);
})();
