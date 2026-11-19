// hyt-gui page. All settings logic runs in Rust (hyt-core); this file only renders what the
// commands return and sends edits back. Text from the radio is only ever set via textContent.

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const { open: pickPath } = window.__TAURI__.dialog;

const state = {
  info: null,      // app_info
  overview: null,  // open configuration (null when nothing is open)
  view: 'digital', // table id (incl. 'list:<file>'), 'settings' or 'changes'
  table: null,     // current table
  changes: [],     // change list (changes view)
  selected: null,  // selected row index in state.table
  detail: null,    // detail fields of the selected row
  channelOptions: [], // channels that can be put into a zone (zones view)
  settingsIndex: [],  // settings topics
  settingsTopic: null, // selected topic id
  settingsQuery: '',   // search text
  settingsData: [],    // topics shown (one, or search results)
  busy: false,     // a radio operation is running
};

// ---------- small helpers ----------

const $ = (id) => document.getElementById(id);

function h(tag, attrs = {}, ...children) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (v == null || v === false) continue;
    if (k === 'class') el.className = v;
    else if (k.startsWith('on')) el.addEventListener(k.slice(2), v);
    else if (v === true) el.setAttribute(k, '');
    else el.setAttribute(k, v);
  }
  for (const c of children.flat()) {
    if (c == null || c === false) continue;
    el.append(c instanceof Node ? c : String(c));
  }
  return el;
}

function setStatus(text, kind = '') {
  const s = $('status');
  s.textContent = text || '';
  s.className = kind;
}

function errorText(e) {
  return typeof e === 'string' ? e : (e && e.message) || String(e);
}

/** A message box; resolves true for OK. `cancel: false` hides the Cancel button. */
function ask(title, body, { ok = 'OK', cancel = 'Cancel', danger = false } = {}) {
  const dlg = $('dlg-message');
  $('msg-title').textContent = title;
  const b = $('msg-body');
  b.replaceChildren(...(Array.isArray(body) ? body : [h('p', {}, body)]));
  $('msg-ok').textContent = ok;
  $('msg-ok').className = danger ? 'danger primary' : 'primary';
  $('msg-cancel').hidden = !cancel;
  if (cancel) $('msg-cancel').textContent = cancel;
  return new Promise((resolve) => {
    dlg.addEventListener('close', () => resolve(dlg.returnValue === 'ok'), { once: true });
    dlg.returnValue = '';
    dlg.showModal();
  });
}

function showDialog(dlg) {
  return new Promise((resolve) => {
    dlg.addEventListener('close', () => resolve(dlg.returnValue), { once: true });
    dlg.returnValue = '';
    dlg.showModal();
  });
}

async function confirmDiscard() {
  const n = state.overview?.changes || 0;
  if (!n) return true;
  return ask('Unsaved changes', `You have ${n} unsaved change${n === 1 ? '' : 's'}. Discard them?`,
    { ok: 'Discard', danger: true });
}

/** Folder picker: chooses the parent folder and keeps the suggested folder name. */
async function browseNewFolder(input) {
  const current = input.value.trim();
  const sep = current.includes('\\') ? '\\' : '/';
  const name = current.split(/[\\/]/).filter(Boolean).pop() || 'hyt-folder';
  const parent = await pickPath({ directory: true, title: 'Choose where to create the new folder' });
  if (parent) input.value = parent.replace(/[\\/]+$/, '') + sep + name;
}

// ---------- loading ----------

async function loadView() {
  if (!state.overview) return;
  if (state.view === 'changes') {
    state.table = null;
    state.changes = await invoke('get_changes');
    state.detail = null;
  } else if (state.view === 'settings') {
    state.table = null;
    state.detail = null;
    state.settingsIndex = await invoke('get_settings_index');
    if (!state.settingsIndex.some((t) => t.id === state.settingsTopic)) state.settingsTopic = state.settingsIndex[0]?.id || null;
    state.settingsData = state.settingsQuery.trim()
      ? await invoke('search_settings', { query: state.settingsQuery })
      : (state.settingsTopic ? [await invoke('get_settings_topic', { id: state.settingsTopic })] : []);
  } else {
    state.table = await invoke('get_table', { id: state.view });
    if (state.selected != null && state.selected >= state.table.rows.length) state.selected = null;
    state.detail = state.selected != null
      ? await invoke('get_detail', { record: state.table.rows[state.selected].record })
      : null;
    state.channelOptions = state.view === 'zones' && state.selected != null ? await invoke('get_channel_options') : [];
  }
}

async function refresh() {
  try {
    await loadView();
  } catch (e) {
    setStatus(errorText(e), 'error');
  }
  render();
}

async function opened(overview, message) {
  state.overview = overview;
  state.view = 'digital';
  state.selected = null;
  await refresh();
  setStatus(message, 'ok');
}

async function openFolder(path) {
  try {
    const o = await invoke('open_folder', { path });
    await opened(o, `Opened ${path}`);
  } catch (e) {
    setStatus(errorText(e), 'error');
  }
}

/** Run a structural action returning a new overview; shows errors in a dialog. */
async function act(command, args, message) {
  try {
    const result = await invoke(command, args);
    state.overview = Array.isArray(result) ? result[0] : result;
    setStatus(message, 'ok');
    await refresh();
    return result;
  } catch (e) {
    setStatus(errorText(e), 'error');
    await refresh();
    await ask('Not possible', [h('p', {}, errorText(e))], { cancel: false });
    return null;
  }
}

/** Send one edit; returns null on success or the error text. */
async function commit(field, text) {
  try {
    state.overview = await invoke('set_value', { field, text });
    setStatus(`${field.key} set to ${text}`, 'ok');
    await refresh();
    return null;
  } catch (e) {
    const msg = errorText(e);
    setStatus(msg, 'error');
    return msg;
  }
}

// ---------- rendering ----------

function render() {
  renderHeader();
  renderNav();
  renderContent();
  renderDetail();
}

function renderHeader() {
  const o = state.overview;
  const id = $('identity');
  if (!o) {
    id.replaceChildren();
  } else {
    id.replaceChildren(
      h('div', {}, h('strong', {}, o.model || 'Unknown model'), ` · serial ${o.serial || '?'} · firmware ${o.firmware || '?'} · RCDB ${o.rcdb_version}`),
      h('span', { class: 'folder', title: o.folder }, `‎${o.folder}`),
    );
  }
  $('btn-save').disabled = !o || state.busy;
  $('btn-write').disabled = !o || state.busy;
  $('btn-open').disabled = state.busy;
  $('btn-read').disabled = state.busy;

  const banner = $('banner');
  if (state.info && !state.info.rcdb_found) {
    banner.hidden = false;
    banner.className = 'banner error';
    banner.textContent = `Hytera CPS data not found in ${state.info.rcdb_dir}. Install CPS there or set HYT_RCDB to a folder with its RcdbJsons, RCDB\\FileID and RCDB\\TableLink folders.`;
  } else if (o?.options_note) {
    banner.hidden = false;
    banner.className = 'banner';
    banner.textContent = `Values are shown as raw numbers: ${o.options_note}`;
  } else {
    banner.hidden = true;
  }
}

function renderNav() {
  const nav = $('nav');
  const o = state.overview;
  if (!o) {
    nav.replaceChildren();
    return;
  }
  const item = (id, title, extra) => h('button', {
    class: state.view === id ? 'active' : '',
    onclick: async () => {
      state.view = id;
      state.selected = null;
      await refresh();
    },
  }, h('span', {}, title), extra);
  const lists = o.lists || [];
  nav.replaceChildren(
    ...o.tables.map((t) => item(t.id, t.title, h('span', { class: 'count' }, `${t.rows}/${t.capacity}`))),
    ...(o.has_settings ? [item('settings', 'Settings', null)] : []),
    ...(lists.length ? [h('div', { class: 'nav-heading' }, 'Lists')] : []),
    ...lists.map((t) => item(t.id, t.title, h('span', { class: 'count' }, `${t.rows}/${t.capacity}`))),
    h('hr'),
    item('changes', 'Changes', o.changes ? h('span', { class: 'badge' }, o.changes) : h('span', { class: 'count' }, '0')),
  );
}

function renderContent() {
  const c = $('content');
  if (!state.overview) {
    c.replaceChildren(h('div', { class: 'empty' },
      h('h1', {}, 'No configuration open'),
      h('p', {}, 'Open a folder saved by hyt read (or by this app), or read the radio over the programming cable.'),
      h('div', { class: 'buttons' },
        h('button', { onclick: onOpen }, 'Open folder…'),
        h('button', { onclick: onRead }, 'Read radio…')),
    ));
    return;
  }
  if (state.view === 'changes') {
    c.replaceChildren(renderChanges());
    return;
  }
  if (state.view === 'settings') {
    renderSettings(c);
    return;
  }
  const t = state.table;
  if (!t) {
    c.replaceChildren();
    return;
  }
  const kind = t.id === 'digital' ? 'digital' : 'analog';
  const selectedRow = state.selected != null ? t.rows[state.selected] : null;
  const actions = t.resizable ? h('span', { class: 'actions' },
    h('button', { onclick: () => onAddChannel(kind), disabled: t.rows.length >= t.capacity }, 'Add channel'),
    h('button', { class: 'danger', onclick: () => onRemoveChannel(kind, selectedRow), disabled: !selectedRow, title: selectedRow ? null : 'Select a channel first' }, 'Remove channel')) : null;
  const head = h('div', { class: 'view-head' },
    h('h1', {}, t.title),
    h('span', { class: 'note' }, t.rows.length
      ? `${t.rows.length} of ${t.capacity} · double-click a cell to edit · Enter saves, Esc cancels`
      : `0 of ${t.capacity}`),
    actions);
  const note = t.note ? h('p', { class: 'note table-note' }, t.note) : null;
  const table = h('table', { class: 'grid' },
    h('thead', {}, h('tr', {},
      h('th', {}, 'No.'),
      t.columns.map((col) => h('th', { title: col.key }, col.title, col.unit ? h('small', {}, col.unit) : null)))),
    h('tbody', {}, t.rows.map((row, i) => renderRow(t, row, i))));
  c.replaceChildren(...[head, note, table].filter(Boolean));
}

function renderRow(t, row, i) {
  const tr = h('tr', {
    class: [state.selected === i ? 'selected' : '', row.added ? 'added' : ''].join(' ').trim(),
    title: row.added ? 'Added since the folder was opened' : null,
    onclick: async () => {
      if (state.selected === i) return;
      state.selected = i;
      await refresh();
    },
  }, h('td', { class: 'no' }, row.no));
  row.cells.forEach((cell, j) => {
    const col = cell.editor ? { ...t.columns[j], editor: cell.editor } : t.columns[j];
    const classes = [cell.changed ? 'changed' : '', cell.editable ? 'editable' : '', cell.disabled ? 'disabled' : '',
      col.key === 'members' ? 'members' : ''];
    const tip = cell.disabled || (cell.editable ? 'Double-click to edit' : null);
    const td = h('td', { class: classes.join(' ').trim(), title: tip }, cell.text);
    if (cell.editable) td.addEventListener('dblclick', () => startCellEdit(td, col, cell));
    tr.append(td);
  });
  return tr;
}

/**
 * An input for `column`'s editor holding `cell`'s value. `onDone(text|null)` gets the new text on
 * Enter/blur/select, or null on Esc. Returns the element to insert and the control to focus.
 */
function makeEditor(column, cell, onDone) {
  const ed = column.editor;
  let control;
  if (ed.kind === 'choice') {
    control = h('select', {});
    const options = ed.options.includes(cell.text) ? ed.options : [cell.text, ...ed.options];
    for (const o of options) control.append(h('option', { value: o }, o));
    control.value = cell.text;
    control.addEventListener('change', () => onDone(control.value));
    control.addEventListener('keydown', (e) => { if (e.key === 'Escape') onDone(null); });
  } else {
    control = h('input', {
      type: 'text',
      value: cell.text,
      spellcheck: 'false',
      maxlength: ed.kind === 'text' ? ed.max_len : null,
      placeholder: ed.kind === 'number' ? ed.hint : null,
      title: ed.kind === 'number' ? `Allowed: ${ed.hint}` : (ed.kind === 'text' ? `Up to ${ed.max_len} characters` : null),
    });
    control.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') { e.preventDefault(); onDone(control.value); }
      if (e.key === 'Escape') { e.preventDefault(); onDone(null); }
    });
    control.addEventListener('blur', () => onDone(control.value));
  }
  const wrap = column.unit ? h('span', { class: 'value' }, control, h('span', { class: 'unit' }, column.unit)) : control;
  return { wrap, control };
}

function startCellEdit(td, column, cell) {
  if (td.classList.contains('editing')) return;
  td.classList.add('editing');
  let busy = false;
  const { wrap, control } = makeEditor(column, cell, async (text) => {
    if (busy) return;
    if (text == null || text === cell.text) {
      busy = true;
      render();
      return;
    }
    busy = true;
    const err = await commit(cell.field, text);
    if (err) {
      busy = false;
      control.classList.add('invalid');
      control.title = err;
      control.focus();
    }
  });
  td.replaceChildren(wrap);
  control.focus();
  if (control.select) control.select();
}

function renderDetail() {
  const d = $('detail');
  if (!state.overview || state.view === 'changes' || state.selected == null || !state.table || !state.detail) {
    d.replaceChildren();
    return;
  }
  const row = state.table.rows[state.selected];
  const titleCell = row.cells[0];
  const kind = state.table.item || { digital: 'Digital channel', analog: 'Analog channel', zones: 'Zone', contacts: 'Contact' }[state.table.id] || 'Record';
  const parts = [h('div', { class: 'detail-head' }, h('h2', {}, `${kind} ${row.no}`), h('span', { class: 'note' }, titleCell.text))];

  if (row.members) {
    const zone = state.selected;
    const last = row.members.length - 1;
    parts.push(h('h3', {}, `Channels (${row.members.length})`));
    parts.push(h('table', { class: 'grid members-table' },
      h('thead', {}, h('tr', {}, h('th', {}, 'No.'), h('th', {}, 'Alias'), h('th', {}, 'Type'), h('th', {}, ''))),
      h('tbody', {}, row.members.map((m, i) => h('tr', {},
        h('td', { class: 'no' }, m.no), h('td', {}, m.alias), h('td', {}, `${m.kind} ${m.channel}`),
        h('td', { class: 'member-actions' },
          h('button', { class: 'icon', title: 'Move up', disabled: i === 0, onclick: () => act('move_zone_member', { zone, member: i, up: true }, `Moved ${m.alias} up`) }, '↑'),
          h('button', { class: 'icon', title: 'Move down', disabled: i === last, onclick: () => act('move_zone_member', { zone, member: i, up: false }, `Moved ${m.alias} down`) }, '↓'),
          h('button', { class: 'icon danger', title: 'Remove from this zone (the channel itself stays)', onclick: () => act('remove_zone_member', { zone, member: i }, `Removed ${m.alias} from the zone`) }, '✕')))))));
    const inZone = new Set(row.members.map((m) => `${m.kind} ${m.channel}`));
    const available = state.channelOptions.filter((o) => !inZone.has(`${o.kind === 'digital' ? 'Digital' : 'Analog'} ${o.number}`));
    const select = h('select', { disabled: !available.length },
      available.length ? available.map((o, k) => h('option', { value: k }, o.label)) : h('option', {}, 'Every channel is in this zone'));
    parts.push(h('div', { class: 'add-member' }, select,
      h('button', { disabled: !available.length, onclick: () => {
        const o = available[Number(select.value)];
        act('add_zone_member', { zone, kind: o.kind, channel: o.number }, `Added ${o.label} to the zone`);
      } }, 'Add to zone')));
  }

  if (row.list_members) {
    // Scan, roam and RX group lists: members by name, read-only.
    const title = state.table.columns[state.table.columns.length - 1]?.title || 'Members';
    parts.push(h('h3', {}, `${title} (${row.list_members.length})`));
    parts.push(row.list_members.length
      ? h('table', { class: 'grid members-table' },
        h('thead', {}, h('tr', {}, h('th', {}, 'No.'), h('th', {}, 'Name'), h('th', {}, 'Refers to'))),
        h('tbody', {}, row.list_members.map((m) => h('tr', {},
          h('td', { class: 'no' }, m.no), h('td', {}, m.name), h('td', {}, m.refers_to)))))
      : h('p', { class: 'note' }, 'No members.'));
    parts.push(h('p', { class: 'note' }, 'Members are shown read-only: changing them needs a recording of CPS doing it first.'));
  }

  parts.push(h('h3', {}, 'All settings'));
  const grid = h('div', { class: 'fields' });
  for (const field of state.detail) {
    const f = field.cell.editor ? { ...field, column: { ...field.column, editor: field.cell.editor } } : field;
    const label = h('span', { class: `label${f.cell.disabled ? ' disabled' : ''}`, title: f.cell.disabled || f.column.key }, f.column.title);
    const value = h('span', { class: `value${f.cell.changed ? ' changed' : ''}${f.cell.disabled ? ' disabled' : ''}` });
    if (f.cell.editable && f.column.editor.kind !== 'read_only') {
      let sending = false;
      const { wrap, control } = makeEditor(f.column, f.cell, async (text) => {
        if (sending || text == null || text === f.cell.text) {
          if (text == null) control.value = f.cell.text;
          return;
        }
        sending = true;
        const err = await commit(f.cell.field, text);
        sending = false;
        if (err) {
          control.classList.add('invalid');
          control.title = err;
        }
      });
      value.append(...(wrap === control ? [control] : wrap.childNodes));
    } else {
      value.append(h('span', { class: 'ro', title: f.cell.disabled || f.cell.text }, f.cell.text));
      if (f.column.unit) value.append(h('span', { class: 'unit' }, f.column.unit));
    }
    grid.append(label, value);
  }
  parts.push(grid);
  d.replaceChildren(...parts);
}

/** Settings view: topic list + form. The search box survives re-rendering (keeps focus). */
function renderSettings(c) {
  let view = document.getElementById('settings-view');
  if (!view || !c.contains(view)) {
    const search = h('input', { id: 'settings-search', type: 'search', placeholder: 'Search all settings…', spellcheck: 'false', value: state.settingsQuery });
    let timer = null;
    search.addEventListener('input', () => {
      clearTimeout(timer);
      timer = setTimeout(async () => {
        state.settingsQuery = search.value;
        await refresh();
      }, 200);
    });
    view = h('div', { id: 'settings-view' },
      h('div', { class: 'view-head' }, h('h1', {}, 'Settings'), search),
      h('div', { class: 'settings-layout' }, h('div', { id: 'settings-topics' }), h('div', { id: 'settings-body' })));
    c.replaceChildren(view);
  }
  const searching = state.settingsQuery.trim() !== '';
  document.getElementById('settings-topics').replaceChildren(...state.settingsIndex.map((t) => h('button', {
    class: !searching && t.id === state.settingsTopic ? 'active' : '',
    onclick: async () => {
      state.settingsTopic = t.id;
      state.settingsQuery = '';
      document.getElementById('settings-search').value = '';
      await refresh();
      document.getElementById('settings-body').scrollTop = 0;
    },
  }, h('span', {}, t.title), t.changed ? h('span', { class: 'badge' }, t.changed) : (t.read_only ? h('span', { class: 'count', title: 'Read-only' }, '🔒') : null))));
  const body = document.getElementById('settings-body');
  if (searching && !state.settingsData.length) {
    body.replaceChildren(h('p', { class: 'note' }, `No setting matches "${state.settingsQuery}".`));
    return;
  }
  body.replaceChildren(...state.settingsData.map((t) => h('div', { class: 'settings-topic' },
    h('h2', {}, t.title),
    t.note ? h('p', { class: 'note' }, t.note) : null,
    ...t.sections.map((s) => h('section', { class: 'settings-section' },
      h('h3', {}, s.title),
      ...s.groups.map((g) => h('div', { class: 'settings-group' },
        g.title ? h('h4', {}, g.title) : null,
        renderSettingRows(g.settings))))))));
}

function renderSettingRows(settings) {
  const grid = h('div', { class: 'fields' });
  for (const s of settings) {
    const label = h('span', { class: `label${s.cell.disabled ? ' disabled' : ''}`, title: s.cell.disabled || s.column.key }, s.label);
    const value = h('span', { class: `value${s.cell.changed ? ' changed' : ''}${s.cell.disabled ? ' disabled' : ''}` });
    if (s.cell.editable && s.column.editor.kind !== 'read_only') {
      let sending = false;
      const { wrap, control } = makeEditor(s.column, s.cell, async (text) => {
        if (sending || text == null || text === s.cell.text) {
          if (text == null) control.value = s.cell.text;
          return;
        }
        sending = true;
        const err = await commit(s.cell.field, text);
        sending = false;
        if (err) {
          control.classList.add('invalid');
          control.title = err;
        }
      });
      value.append(...(wrap === control ? [control] : [...wrap.childNodes]));
    } else {
      value.append(h('span', { class: 'ro', title: s.read_only || s.cell.disabled || s.cell.text }, s.cell.text));
      if (s.column.unit) value.append(h('span', { class: 'unit' }, s.column.unit));
    }
    grid.append(label, value);
  }
  return grid;
}

function renderChanges() {
  const list = state.changes;
  const head = h('div', { class: 'view-head' },
    h('h1', {}, 'Changes'),
    h('span', { class: 'note' }, list.length
      ? `${list.length} setting${list.length === 1 ? '' : 's'} differ from ${state.overview.folder}`
      : 'No changes from the opened folder.'),
    list.length ? h('span', { class: 'actions' }, h('button', { class: 'danger', onclick: onDiscard }, 'Discard all')) : null);
  if (!list.length) return head;
  const table = h('table', { class: 'grid changes' },
    h('thead', {}, h('tr', {}, h('th', {}, 'Where'), h('th', {}, 'Setting'), h('th', {}, 'Before'), h('th', {}, 'After'), h('th', {}, ''))),
    h('tbody', {}, list.map((c) => h('tr', {},
      h('td', {}, c.place),
      h('td', {}, c.setting),
      h('td', {}, c.old),
      h('td', { class: 'changed' }, c.new),
      h('td', {}, c.field ? h('button', { class: 'link', onclick: () => onRevert(c.field) }, 'Revert') : null)))));
  return h('div', {}, head, table);
}

// ---------- actions ----------

async function onOpen() {
  if (!(await confirmDiscard())) return;
  const path = await pickPath({ directory: true, title: 'Open a folder saved by hyt read' });
  if (path) await openFolder(path);
}

async function onAddChannel(kind) {
  const result = await act('add_channel', { kind }, `Added a ${kind} channel from CPS's channel template; rename it and set its frequencies`);
  if (result) {
    state.selected = result[1];
    await refresh();
  }
}

async function onRemoveChannel(kind, row) {
  if (!row) return;
  const name = row.cells[0].text;
  const ok = await ask(`Remove ${kind} channel ${row.no}`,
    [h('p', {}, `Remove "${name}"? Zone members pointing to it are removed and later channels move up, as CPS does.`),
     h('p', { class: 'note' }, 'Channels used elsewhere (scan or roam lists, emergency revert, current or power-on channel, …) cannot be removed yet.')],
    { ok: 'Remove', danger: true });
  if (!ok) return;
  const result = await act('remove_channel', { kind, position: row.no - 1 }, `Removed ${kind} channel "${name}"`);
  if (result) {
    state.selected = null;
    await refresh();
  }
}

async function onRevert(field) {
  try {
    state.overview = await invoke('revert_value', { field });
    setStatus(`Reverted ${field.key}`, 'ok');
  } catch (e) {
    setStatus(errorText(e), 'error');
  }
  await refresh();
}

async function onDiscard() {
  if (!(await ask('Discard all changes', 'Undo every change since the folder was opened or saved?', { ok: 'Discard', danger: true }))) return;
  state.overview = await invoke('discard_changes');
  setStatus('All changes discarded', 'ok');
  await refresh();
}

async function onSave() {
  $('save-out').value = await invoke('suggest_folder', { kind: 'edited' });
  if ((await showDialog($('dlg-save'))) !== 'go') return;
  const path = $('save-out').value.trim();
  try {
    const n = state.overview.changes;
    state.overview = await invoke('save_as', { path });
    setStatus(`Saved ${n ? `${n} change${n === 1 ? '' : 's'}` : 'the configuration'} into ${path}`, 'ok');
  } catch (e) {
    setStatus(errorText(e), 'error');
  }
  await refresh();
}

async function fillPorts(select) {
  const previous = select.value;
  let ports = [];
  try {
    ports = await invoke('list_ports');
  } catch (e) {
    setStatus(errorText(e), 'error');
  }
  select.replaceChildren(...ports.map((p) => h('option', { value: p.name }, p.description ? `${p.name} — ${p.description}` : p.name)));
  if (state.info?.sim) select.append(h('option', { value: '__sim__' }, `Simulated radio (${state.info.sim})`));
  if (state.info?.dev) select.append(h('option', { value: '__replay__' }, 'Replay a recording (testing)'));
  if (!select.options.length) select.append(h('option', { value: '' }, 'No serial ports found'));
  const preferred = ports.find((p) => p.name === previous) || ports.find((p) => /prolific|usb/i.test(p.description)) || ports[0];
  if (previous === '__sim__' || (state.info?.sim && !preferred)) select.value = '__sim__';
  else if (preferred) select.value = preferred.name;
  select.dispatchEvent(new Event('change'));
}

/** `{port}` or, in dev mode, `{replay}` for the chosen entry of a port list. */
async function chosenSource(select, replayInput) {
  if (select.value === '__sim__') return 'sim';
  if (select.value === '__replay__') {
    if (!replayInput.value) throw new Error('choose a recording to replay');
    return { replay: replayInput.value };
  }
  if (!select.value) throw new Error('no serial port selected');
  return { port: select.value };
}

/** Run a radio operation with the progress dialog open. */
async function withProgress(title, op) {
  const dlg = $('dlg-progress');
  $('prog-title').textContent = title;
  $('prog-stage').textContent = 'Starting';
  $('prog-bar').removeAttribute('value');
  state.busy = true;
  renderHeader();
  dlg.showModal();
  try {
    return await op();
  } finally {
    dlg.close();
    state.busy = false;
    renderHeader();
  }
}

async function onRead() {
  if (state.busy || !(await confirmDiscard())) return;
  $('read-out').value = await invoke('suggest_folder', { kind: 'read' });
  await fillPorts($('read-port'));
  if ((await showDialog($('dlg-read'))) !== 'go') return;
  const out = $('read-out').value.trim();
  let source;
  try {
    source = await chosenSource($('read-port'), $('read-replay'));
  } catch (e) {
    setStatus(errorText(e), 'error');
    return;
  }
  try {
    const o = await withProgress('Reading the radio', () => invoke('read_radio', { source, out }));
    await opened(o, `Read the radio into ${out}`);
  } catch (e) {
    setStatus(`Read failed: ${errorText(e)}`, 'error');
    await ask('Read failed', [h('pre', {}, errorText(e))], { cancel: false });
  }
}

async function onWrite() {
  if (state.busy) return;
  const o = state.overview;
  const changes = await invoke('get_changes');
  const summary = $('write-summary');
  summary.replaceChildren(
    h('div', { class: 'kv' },
      h('span', {}, 'Radio'), h('span', {}, `${o.model || '?'} · serial ${o.serial || '?'} · RCDB ${o.rcdb_version}`),
      h('span', {}, 'Configuration'), h('span', {}, o.folder),
      h('span', {}, 'Changes'), h('span', {}, changes.length
        ? `${changes.length} setting${changes.length === 1 ? '' : 's'} changed from the opened folder (listed below)`
        : 'none: the opened folder is written as it is')),
    changes.length ? h('div', { class: 'change-list' }, h('table', {}, h('tbody', {},
      changes.map((c) => h('tr', {}, h('td', {}, c.place), h('td', {}, c.setting), h('td', {}, `${c.old} → ${c.new}`)))))) : null,
  );
  $('write-save-row').hidden = !changes.length;
  if (changes.length) $('write-save').value = await invoke('suggest_folder', { kind: 'edited' });
  $('write-backup').value = await invoke('suggest_folder', { kind: 'backup' });
  $('write-confirm').checked = false;
  $('write-go').disabled = true;
  await fillPorts($('write-port'));
  if ((await showDialog($('dlg-write'))) !== 'go') return;

  let port;
  try {
    port = await chosenSource($('write-port'), $('read-replay'));
  } catch (e) {
    setStatus(errorText(e), 'error');
    return;
  }
  const backup = $('write-backup').value.trim();
  const saveTo = changes.length ? $('write-save').value.trim() : null;
  try {
    const r = await withProgress('Writing to the radio', () => invoke('write_radio', { port, backup, saveTo }));
    state.overview = await invoke('get_overview');
    await refresh();
    setStatus(`Written and verified: ${r.written} files (${r.differing} changed). Backup in ${r.backup}`, 'ok');
    await ask('Write verified', [
      h('p', {}, `All ${r.written} written files read back as written (${r.differing} differed from what the radio had).`),
      h('div', { class: 'kv' }, h('span', {}, 'Written from'), h('span', {}, r.folder), h('span', {}, 'Backup before writing'), h('span', {}, r.backup)),
    ], { cancel: false });
  } catch (e) {
    state.overview = await invoke('get_overview');
    await refresh();
    setStatus(`Write failed: ${errorText(e)}`, 'error');
    await ask('Write failed', [
      h('pre', {}, errorText(e)),
      h('p', {}, `If a backup was taken it is in ${backup}. It can be written back with hyt write or this app.`),
    ], { cancel: false });
  }
}

// ---------- wiring ----------

async function init() {
  state.info = await invoke('app_info');

  $('btn-open').addEventListener('click', onOpen);
  $('btn-read').addEventListener('click', onRead);
  $('btn-save').addEventListener('click', onSave);
  $('btn-write').addEventListener('click', onWrite);
  $('read-refresh').addEventListener('click', () => fillPorts($('read-port')));
  $('write-refresh').addEventListener('click', () => fillPorts($('write-port')));
  $('read-out-browse').addEventListener('click', () => browseNewFolder($('read-out')));
  $('save-browse').addEventListener('click', () => browseNewFolder($('save-out')));
  $('write-save-browse').addEventListener('click', () => browseNewFolder($('write-save')));
  $('write-backup-browse').addEventListener('click', () => browseNewFolder($('write-backup')));
  $('write-confirm').addEventListener('change', (e) => { $('write-go').disabled = !e.target.checked; });
  $('read-port').addEventListener('change', () => { $('read-replay-row').hidden = $('read-port').value !== '__replay__'; });
  $('read-replay-browse').addEventListener('click', async () => {
    const p = await pickPath({ title: 'Recording to replay', filters: [{ name: 'Recording', extensions: ['jsonl'] }] });
    if (p) $('read-replay').value = p;
  });

  await listen('progress', (e) => {
    const p = e.payload;
    $('prog-stage').textContent = p.stage;
    const bar = $('prog-bar');
    if (p.total > 0) {
      bar.max = p.total;
      bar.value = p.done;
    } else {
      bar.removeAttribute('value');
    }
  });

  const win = window.__TAURI__.window.getCurrentWindow();
  await win.onCloseRequested(async (event) => {
    if (state.busy) {
      event.preventDefault();
      await ask('Radio operation running', 'Wait until reading or writing has finished before closing.', { cancel: false });
      return;
    }
    if (!(await confirmDiscard())) event.preventDefault();
  });

  const existing = await invoke('get_overview');
  if (existing) {
    await opened(existing, `Opened ${existing.folder}`);
  } else if (state.info.startup_folder) {
    await openFolder(state.info.startup_folder);
  } else {
    render();
  }
}

init().catch((e) => setStatus(errorText(e), 'error'));
