// hyt-gui page. All settings logic runs in Rust (hyt-core); this file only renders what the
// commands return and sends edits back. Text from the radio is only ever set via textContent.

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
// Looked up on each call (not at load) so a scripted test can stand in for the native dialogs.
const pickPath = (options) => window.__TAURI__.dialog.open(options);
const pickSavePath = (options) => window.__TAURI__.dialog.save(options);

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
  helpOpen: new Set(), // fields whose help text is expanded (file:block:key)
  showLicence: false,  // show (read-only) fields of features the radio is not licensed for
  showInternal: false, // show (read-only) fields Hytera hides even in CPS
  quick: null,     // quick setup: questions, answers and the step shown (null until opened)
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

// ---------- help texts (CPS's own, shown on request) ----------

/** CPS's description, choices and notes for a field. */
function helpContent(help) {
  return h('div', { class: 'help-text' },
    help.text ? h('p', {}, help.text) : null,
    help.options ? h('div', {}, h('strong', {}, 'Choices'), h('p', {}, help.options)) : null,
    help.notes ? h('div', {}, h('strong', {}, 'Note'), h('p', {}, help.notes)) : null,
    h('p', { class: 'help-source' }, 'Description from Hytera CPS'));
}

const helpKey = (field, column) => (field ? `${field.file}:${field.block}:${field.key}` : `column:${column.key}`);

/**
 * Append `label`, `value` and (when the field has help) a collapsed help box to a `.fields` grid,
 * with a "?" button in the label that shows or hides it. Open boxes stay open across re-renders.
 */
function appendField(grid, label, value, help, key) {
  if (!help) {
    grid.append(label, value);
    return;
  }
  const box = helpContent(help);
  box.classList.add('inline');
  box.hidden = !state.helpOpen.has(key);
  const button = h('button', {
    type: 'button',
    class: `help-btn${box.hidden ? '' : ' open'}`,
    title: 'What is this? (description from CPS)',
    'aria-expanded': String(!box.hidden),
    onclick: (e) => {
      e.preventDefault();
      e.stopPropagation();
      box.hidden = !box.hidden;
      if (box.hidden) state.helpOpen.delete(key); else state.helpOpen.add(key);
      button.classList.toggle('open', !box.hidden);
      button.setAttribute('aria-expanded', String(!box.hidden));
    },
  }, '?');
  label.append(button);
  grid.append(label, value, box);
}

/** A field label: the name (shortened with … when long) and room for a help button. */
function fieldLabel(text, { disabled = false, title = null } = {}) {
  return h('span', { class: `label${disabled ? ' disabled' : ''}`, title }, h('span', { class: 'label-text' }, text));
}

// ---------- fields CPS hides (licence-limited or internal), shown on request ----------

const HIDDEN_PREFS = { licence: 'hyt.showLicence', internal: 'hyt.showInternal' };

function loadHiddenPrefs() {
  try {
    state.showLicence = localStorage.getItem(HIDDEN_PREFS.licence) === '1';
    state.showInternal = localStorage.getItem(HIDDEN_PREFS.internal) === '1';
  } catch { /* storage unavailable: keep the defaults */ }
}

function setHiddenPref(kind, on) {
  if (kind === 'licence') state.showLicence = on; else state.showInternal = on;
  try { localStorage.setItem(HIDDEN_PREFS[kind], on ? '1' : '0'); } catch { /* not remembered */ }
  syncHiddenMenu();
  // Leave a list that is now hidden as a whole.
  if (kind === 'licence' && !on && state.overview?.lists?.some((l) => l.id === state.view && l.hidden)) {
    state.view = 'digital';
    state.selected = null;
    refresh();
    return;
  }
  render();
}

function syncHiddenMenu() {
  for (const [id, on] of [['mi-show-licence', state.showLicence], ['mi-show-internal', state.showInternal]]) {
    const item = $(id);
    item.setAttribute('aria-checked', String(on));
    item.classList.toggle('checked', on);
  }
}

/** Whether a field with this column is shown under the current switches. */
function fieldShown(column) {
  const hidden = column?.hidden;
  if (!hidden) return true;
  return hidden.kind === 'licence' ? state.showLicence : state.showInternal;
}

/** A tag after the label of a field CPS hides, with the reason as tooltip. */
function hiddenTag(column) {
  const why = column?.hidden;
  if (!why) return null;
  return why.kind === 'licence'
    ? h('span', { class: 'tag licence', title: why.reason }, 'licence')
    : h('span', { class: 'tag internal', title: why.reason }, 'internal');
}

/** "12 fields hidden like CPS: 8 need a licence · 4 internal", with links that show them. */
function hiddenNote(columns) {
  const count = (kind) => columns.filter((c) => c?.hidden?.kind === kind).length;
  const licence = count('licence');
  const internal = count('internal');
  if (!licence && !internal) return null;
  const link = (kind, n, label, on) => n ? h('button', { class: 'link', onclick: () => setHiddenPref(kind, !on) },
    on ? `hide the ${label}` : `show ${n} ${n === 1 ? label.replace(/s$/, '') : label}`) : null;
  const parts = [link('licence', licence, 'fields that need a licence', state.showLicence), link('internal', internal, 'internal fields', state.showInternal)].filter(Boolean);
  return h('p', { class: 'note hidden-note' },
    `${plural(licence + internal, 'field')} hidden like CPS does: `, ...parts.flatMap((p, i) => (i ? [' · ', p] : [p])));
}

/** Show `help` in a small popover under `anchor` (table column headers). */
function showHelpPopover(anchor, title, help) {
  const pop = $('help-pop');
  pop.replaceChildren(h('div', { class: 'help-pop-title' }, title), helpContent(help));
  pop.hidden = false;
  const r = anchor.getBoundingClientRect();
  const width = pop.offsetWidth;
  pop.style.left = `${Math.max(8, Math.min(r.left, window.innerWidth - width - 8))}px`;
  pop.style.top = `${r.bottom + 6}px`;
}

function hideHelpPopover() {
  $('help-pop').hidden = true;
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
  } else if (state.view === 'quick') {
    state.table = null;
    state.detail = null;
    // Questions are rebuilt each time (what the radio has now may have changed); answers stay.
    const questions = await invoke('get_quick_setup');
    state.quick = state.quick
      ? { ...state.quick, questions }
      : { questions, step: 0, mode: null, answers: {}, values: {}, channel: null, replace: false, review: null, error: null, openDetails: new Set() };
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
  state.quick = null;
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
  $('btn-more').disabled = !o || state.busy;
  if ($('btn-more').disabled) closeMenu();

  const banner = $('banner');
  if (state.info && !state.info.rcdb_found) {
    banner.hidden = false;
    banner.className = 'banner error';
    banner.textContent = `Hytera CPS data not found in ${state.info.rcdb_dir}. Install CPS there or set HYT_RCDB to a folder with its RcdbJsons, RCDB\\FileID and RCDB\\TableLink folders.`;
  } else if (o?.options_note) {
    banner.hidden = false;
    banner.className = 'banner';
    banner.textContent = `Values are shown as raw numbers: ${o.options_note}`;
  } else if (o?.model_note) {
    banner.hidden = false;
    banner.className = 'banner';
    banner.textContent = `Model number: ${o.model_note}`;
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
  // Lists CPS hides as a whole (a licence the radio does not have) only with the licence switch on.
  const lists = (o.lists || []).filter((t) => !t.hidden || state.showLicence);
  nav.replaceChildren(
    item('quick', 'Quick setup', null),
    h('hr'),
    ...o.tables.map((t) => item(t.id, t.title, h('span', { class: 'count' }, `${t.rows}/${t.capacity}`))),
    ...(o.has_settings ? [item('settings', 'Settings', null)] : []),
    ...(lists.length ? [h('div', { class: 'nav-heading' }, 'Lists')] : []),
    ...lists.map((t) => item(t.id, t.title, t.hidden
      ? h('span', { class: 'tag licence', title: t.hidden.reason }, 'licence')
      : h('span', { class: 'count' }, `${t.rows}/${t.capacity}`))),
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
  if (state.view === 'quick') {
    c.replaceChildren(renderQuick());
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
    h('button', { onclick: () => onDuplicateChannel(kind, selectedRow), disabled: !selectedRow || t.rows.length >= t.capacity, title: selectedRow ? 'Add a copy of the selected channel' : 'Select a channel first' }, 'Duplicate'),
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
      t.columns.filter(fieldShown).map((col) => h('th', { title: col.hidden?.reason || col.key },
        h('span', { class: 'th-title' }, col.title, hiddenTag(col),
          col.help ? h('button', {
            type: 'button',
            class: 'help-btn',
            title: 'What is this? (description from CPS)',
            onclick: (e) => { e.stopPropagation(); showHelpPopover(e.currentTarget, col.title, col.help); },
          }, '?') : null),
        col.unit ? h('small', {}, col.unit) : null)))),
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
    if (!fieldShown(t.columns[j])) return;
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
  parts.push(hiddenNote(state.detail.map((f) => f.column)));
  const grid = h('div', { class: 'fields' });
  for (const field of state.detail) {
    if (!fieldShown(field.column)) continue;
    const f = field.cell.editor ? { ...field, column: { ...field.column, editor: field.cell.editor } } : field;
    const label = fieldLabel(f.column.title, { disabled: !!f.cell.disabled || !!f.column.hidden, title: f.column.hidden?.reason || f.cell.disabled || f.column.key });
    if (f.column.hidden) label.append(hiddenTag(f.column));
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
    appendField(grid, label, value, f.column.help, helpKey(f.cell.field, f.column));
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
  // Drop the fields CPS hides (unless switched on), then groups, sections and topics left empty.
  const all = state.settingsData.flatMap((t) => t.sections.flatMap((s) => s.groups.flatMap((g) => g.settings.map((x) => x.column))));
  const topics = state.settingsData.map((t) => ({
    ...t,
    sections: t.sections.map((s) => ({
      ...s,
      groups: s.groups.map((g) => ({ ...g, settings: g.settings.filter((x) => fieldShown(x.column)) })).filter((g) => g.settings.length),
    })).filter((s) => s.groups.length),
  })).filter((t) => t.sections.length);
  const note = hiddenNote(all);
  if (!topics.length) {
    body.replaceChildren(
      h('p', { class: 'note' }, searching ? `No setting matches "${state.settingsQuery}".` : 'Nothing to show on this page.'),
      note);
    return;
  }
  body.replaceChildren(note || '', ...topics.map((t) => h('div', { class: 'settings-topic' },
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
    const label = fieldLabel(s.label, { disabled: !!s.cell.disabled || !!s.column.hidden, title: s.column.hidden?.reason || s.cell.disabled || s.column.key });
    if (s.column.hidden) label.append(hiddenTag(s.column));
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
    appendField(grid, label, value, s.column.help, helpKey(s.cell.field, s.column));
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

// ---------- quick setup ----------
//
// Step 0 asks how to start (adjust what the radio has, or start fresh with recommended answers and
// the starter channels), then one question per step, then a review that lists exactly what would
// change (a dry run in Rust). Applying turns the answers into ordinary pending changes; nothing is
// written to the radio here.

const KIND_LABEL = { digital: 'Digital', analog: 'Analog' };

/** Step numbers: 0 is the start, questions are 1…n, the review is n + 1. */
const reviewStep = (q) => q.questions.length + 1;
const questionAt = (q, step) => (step >= 1 && step <= q.questions.length ? q.questions[step - 1] : null);

/** The question has an answer that changes something (or would). */
function quickAnswered(q, question) {
  if (question.kind === 'choice') return q.answers[question.id] != null;
  if (question.kind === 'values') return Object.keys(q.values[question.id] || {}).length > 0;
  return q.channel != null || q.replace;
}

/** Choose how to start: preselect the recommended answers and the starter channels, or nothing. */
function quickStart(mode) {
  const q = state.quick;
  q.mode = mode;
  q.answers = {};
  q.channel = null;
  q.replace = false;
  if (mode === 'fresh') {
    for (const question of q.questions) {
      if (question.kind === 'choice' && question.recommended) q.answers[question.id] = question.recommended;
      if (question.kind === 'channel' && question.starter.length) q.replace = true;
    }
  }
  renderContent();
}

/** The answers in `apply_quick_setup`'s form. */
function quickReplies() {
  const q = state.quick;
  const replies = [];
  for (const question of q.questions) {
    if (question.kind === 'choice' && q.answers[question.id] != null) {
      replies.push({ question: question.id, answer: q.answers[question.id] });
    } else if (question.kind === 'values' && Object.keys(q.values[question.id] || {}).length) {
      replies.push({ question: question.id, values: q.values[question.id] });
    } else if (question.kind === 'channel' && q.replace) {
      replies.push({ question: question.id, replace_channels: true });
    } else if (question.kind === 'channel' && q.channel) {
      const c = q.channel;
      replies.push({ question: question.id, channel: {
        kind: c.kind, name: c.name, rx: c.rx, tx: c.repeater ? c.tx : '',
        color_code: c.kind === 'digital' ? c.color_code : null,
        slot: c.kind === 'digital' ? c.slot : null,
        spacing: c.kind === 'analog' ? c.spacing : null,
        tone: c.kind === 'analog' ? c.tone : null,
        tone_rx: c.kind === 'analog' && !!c.tone && c.tone_rx,
        add_to_zone: c.add_to_zone,
      } });
    }
  }
  return replies;
}

/** Show step `i` (the review comes after the last question and previews the result). */
async function quickGo(i) {
  const q = state.quick;
  if (questionAt(q, q.step)?.kind === 'channel' && q.channel && i > q.step) {
    const missing = !q.channel.name.trim() ? 'Give the channel a name.' : (!q.channel.rx.trim() ? "Give the channel's frequency." : null);
    if (missing) {
      q.error = missing;
      renderContent();
      return;
    }
  }
  q.step = i;
  q.review = null;
  q.error = null;
  if (i === reviewStep(q)) {
    try {
      const r = await invoke('apply_quick_setup', { replies: quickReplies(), dryRun: true });
      q.review = r.outcome;
    } catch (e) {
      q.error = errorText(e);
    }
  }
  renderContent();
  $('content').scrollTop = 0;
}

async function quickApply() {
  const q = state.quick;
  try {
    const r = await invoke('apply_quick_setup', { replies: quickReplies(), dryRun: false });
    const n = r.outcome.changes.length;
    state.overview = r.overview;
    state.quick = null;
    state.view = 'changes';
    await refresh();
    setStatus(`Quick setup: ${plural(n, 'change')} added to the pending changes. Check them here, then use Save as… or Write to radio….`, 'ok');
  } catch (e) {
    q.error = errorText(e);
    renderContent();
  }
}

function renderQuick() {
  const q = state.quick;
  if (!q) return h('div');
  if (!q.questions.length) {
    return h('div', { class: 'empty' }, h('h1', {}, 'Quick setup'),
      h('p', {}, "None of the quick setup questions apply to this radio, or CPS's screen data is missing (run tools/export-rcdb-options.ps1)."));
  }
  const last = reviewStep(q);
  const steps = h('ol', { class: 'quick-steps' },
    h('li', { class: [q.step === 0 ? 'active' : '', q.mode ? 'done' : ''].join(' ').trim() },
      h('button', { onclick: () => quickGo(0) }, 'Start')),
    ...q.questions.map((question, i) => h('li', {
      class: [i + 1 === q.step ? 'active' : '', quickAnswered(q, question) ? 'done' : ''].join(' ').trim(),
    }, h('button', { onclick: () => quickGo(i + 1) }, question.title))),
    h('li', { class: q.step === last ? 'active' : '' }, h('button', { onclick: () => quickGo(last) }, 'Review')));
  const question = questionAt(q, q.step);
  const body = q.step === 0 ? renderQuickStart() : (question ? renderQuickQuestion(question) : renderQuickReview());
  return h('div', { id: 'quick-view' },
    h('div', { class: 'view-head' }, h('h1', {}, 'Quick setup'),
      h('span', { class: 'note' }, 'A few plain questions. Each answer sets several settings to common values; anything you skip stays as it is.')),
    h('div', { class: 'quick-layout' }, steps, h('div', { class: 'quick-body' }, body)));
}

function quickButtons(nextLabel = 'Next') {
  const q = state.quick;
  return h('div', { class: 'quick-buttons' },
    h('button', { disabled: q.step === 0, onclick: () => quickGo(q.step - 1) }, 'Back'),
    h('button', { class: 'primary', onclick: () => quickGo(q.step + 1) }, nextLabel));
}

function renderQuickQuestion(question) {
  const q = state.quick;
  let body;
  if (question.kind === 'choice') body = renderQuickChoice(question);
  else if (question.kind === 'values') body = renderQuickValues(question);
  else body = renderQuickChannel(question);
  return h('section', { class: 'quick-card' },
    h('h2', {}, question.title),
    h('p', { class: 'quick-intro' }, question.intro),
    body,
    question.note ? h('p', { class: 'quick-note' }, question.note) : null,
    q.error ? h('p', { class: 'quick-error' }, q.error) : null,
    quickButtons(q.step === q.questions.length ? 'Review' : 'Next'));
}

function renderQuickStart() {
  const q = state.quick;
  const channels = q.questions.find((x) => x.kind === 'channel');
  const canReplace = !!channels?.starter.length;
  const option = (mode, label, detail) => h('div', { class: `quick-option${q.mode === mode ? ' chosen' : ''}` },
    h('label', { class: 'quick-option-main' },
      h('input', { type: 'radio', name: 'quick-start', checked: q.mode === mode, onchange: () => quickStart(mode) }),
      h('span', { class: 'quick-option-text' }, h('span', { class: 'quick-option-label' }, label), h('span', { class: 'note' }, detail))));
  return h('section', { class: 'quick-card' },
    h('h2', {}, 'How do you want to start?'),
    h('div', { class: 'quick-options' },
      option('adjust', 'Adjust what the radio has now', 'Every question starts on "Leave as it is": only what you choose changes.'),
      option('fresh', 'Start fresh', canReplace
        ? `Every question starts on the recommended answer, and all channels are replaced by a starter set of ${plural(channels.starter.length, 'channel')}. You can still change each answer.`
        : 'Every question starts on the recommended answer. You can still change each answer.')),
    q.mode ? h('p', { class: 'note' }, 'Changing this resets your answers to the chosen starting point.') : null,
    h('p', { class: 'quick-note' }, 'Settings the questions do not cover stay as they are. Nothing is changed until you apply the review, and nothing is written to the radio until you use Write to radio….'),
    h('div', { class: 'quick-buttons' }, h('button', { class: 'primary', disabled: !q.mode, onclick: () => quickGo(1) }, 'Next')));
}

function renderQuickChoice(question) {
  const q = state.quick;
  const chosen = q.answers[question.id] ?? null;
  const name = `quick-${question.id}`;
  const current = question.answers.find((a) => a.id === question.current);
  const option = (value, label, detail, settings, isNow, isRecommended = false) => {
    const input = h('input', { type: 'radio', name, checked: chosen === value,
      onchange: () => {
        if (value == null) delete q.answers[question.id]; else q.answers[question.id] = value;
        renderContent();
      } });
    const parts = [h('label', { class: 'quick-option-main' }, input,
      h('span', { class: 'quick-option-text' },
        h('span', { class: 'quick-option-label' }, label,
          isNow ? h('span', { class: 'pill', title: 'The radio is set like this now' }, 'now') : null,
          isRecommended ? h('span', { class: 'pill recommended', title: 'hyt\'s suggestion; preselected when starting fresh' }, 'recommended') : null),
        detail ? h('span', { class: 'note' }, detail) : null))];
    if (settings?.length) {
      const key = `${question.id}/${value}`;
      const details = h('details', { class: 'quick-changes', open: q.openDetails.has(key) },
        h('summary', {}, 'What this sets'));
      details.addEventListener('toggle', () => { if (details.open) q.openDetails.add(key); else q.openDetails.delete(key); });
      const grid = h('div', { class: 'fields' });
      for (const s of settings) {
        appendField(grid, fieldLabel(s.setting, { title: s.setting }), h('span', { class: 'value' }, h('span', { class: 'ro' }, s.value)),
          s.help, `quick:${question.id}:${value}:${s.setting}`);
      }
      details.append(grid);
      parts.push(details);
    }
    return h('div', { class: `quick-option${chosen === value ? ' chosen' : ''}` }, ...parts);
  };
  return h('div', { class: 'quick-options' },
    option(null, 'Leave as it is', current ? `Now: ${current.label}` : 'Keep what the radio has now.', null, false),
    ...question.answers.map((a) => option(a.id, a.label, a.detail, a.settings, a.id === question.current, a.id === question.recommended)));
}

/** An input or select for a quick setup value; `onChange(text)` on every change. */
function quickEditor(editor, value, onChange, extra = {}) {
  if (editor.kind === 'choice') {
    const options = editor.options.includes(value) || value === '' ? editor.options : [value, ...editor.options];
    const select = h('select', extra, ...options.map((o) => h('option', { value: o }, o)));
    select.value = value;
    select.addEventListener('change', () => onChange(select.value));
    return select;
  }
  const input = h('input', {
    type: 'text', value, spellcheck: 'false',
    maxlength: editor.kind === 'text' ? editor.max_len : null,
    placeholder: editor.kind === 'number' ? editor.hint : null,
    ...extra,
  });
  input.addEventListener('input', () => onChange(input.value));
  return input;
}

function renderQuickValues(question) {
  const q = state.quick;
  const values = (q.values[question.id] ||= {});
  const grid = h('div', { class: 'fields quick-fields' });
  for (const f of question.fields) {
    const value = h('span', { class: 'value' },
      quickEditor(f.column.editor, values[f.id] ?? f.cell.text, (text) => {
        if (text === f.cell.text) delete values[f.id]; else values[f.id] = text;
      }),
      f.column.unit ? h('span', { class: 'unit' }, f.column.unit) : null);
    appendField(grid, fieldLabel(f.label, { title: `CPS: ${f.column.title}` }), value, f.column.help, helpKey(f.cell.field, f.column));
    if (f.hint) grid.append(h('p', { class: 'note field-hint' }, f.hint));
  }
  return grid;
}

function renderQuickChannel(question) {
  const q = state.quick;
  const form = question;
  const c = q.channel;
  const mode = q.replace ? 'replace' : (c ? 'add' : 'none');
  const pick = (m) => {
    q.replace = m === 'replace';
    q.channel = m === 'add' ? {
      kind: form.kinds[0], name: '', rx: '', tx: '', repeater: false, color_code: '1', slot: form.slots[0] || '',
      spacing: form.spacings.includes('12.5') ? '12.5' : (form.spacings[0] || ''), tone: '', tone_rx: false, add_to_zone: !!form.zone,
    } : null;
    q.error = null;
    renderContent();
  };
  const option = (m, label, detail, extra) => h('div', { class: `quick-option${mode === m ? ' chosen' : ''}` },
    h('label', { class: 'quick-option-main' },
      h('input', { type: 'radio', name: 'quick-channel', checked: mode === m, onchange: () => pick(m) }),
      h('span', { class: 'quick-option-text' }, h('span', { class: 'quick-option-label' }, label), detail ? h('span', { class: 'note' }, detail) : null)),
    extra);
  const choice = h('div', { class: 'quick-options' },
    option('none', 'No, keep the channels as they are'),
    form.kinds.length ? option('add', 'Add a channel') : null,
    form.starter.length ? option('replace', 'Replace all channels with a starter set',
      `Your ${plural(form.existing_digital, 'digital channel')} and ${plural(form.existing_analog, 'analog channel')} are replaced by these ${form.starter.length}.`,
      renderStarter(form)) : null);
  if (!c) return choice;

  const grid = h('div', { class: 'fields quick-fields' });
  const row = (label, control, hint) => {
    grid.append(fieldLabel(label), h('span', { class: 'value' }, ...[control].flat()));
    if (hint) grid.append(h('p', { class: 'note field-hint' }, hint));
  };
  const text = (key, extra = {}) => quickEditor({ kind: 'text', max_len: extra.maxlength }, c[key], (v) => { c[key] = v; }, extra);
  const select = (key, options, rerender = false) => quickEditor({ kind: 'choice', options }, c[key], (v) => { c[key] = v; if (rerender) renderContent(); });
  const check = (key, label) => h('label', { class: 'check' },
    h('input', { type: 'checkbox', checked: !!c[key], onchange: (e) => { c[key] = e.target.checked; renderContent(); } }), label);

  if (form.kinds.length > 1) {
    row('Type', h('span', { class: 'segmented' }, ...form.kinds.map((k) => h('label', {},
      h('input', { type: 'radio', name: 'quick-channel-kind', checked: c.kind === k, onchange: () => { c.kind = k; renderContent(); } }),
      KIND_LABEL[k]))),
    'Digital (DMR) for other digital radios and DMR repeaters; analog (FM) for older radios and analog repeaters.');
  }
  row('Name', text('name', { maxlength: form.name_max_len, placeholder: 'e.g. Base, Club, Simplex 1' }));
  row('Frequency', [text('rx', { placeholder: 'e.g. 439.9875' }), h('span', { class: 'unit' }, 'MHz')], `Allowed: ${form.frequency_hint}`);
  row('Repeater', check('repeater', 'Talk through a repeater (it listens on another frequency)'));
  if (c.repeater) {
    row('Repeater input', [text('tx', { placeholder: 'e.g. 432.3875' }), h('span', { class: 'unit' }, 'MHz')],
      "The frequency the radio transmits on: the repeater's input. The frequency above is the one it listens to.");
  }
  if (c.kind === 'digital') {
    row('Colour code', text('color_code', { placeholder: form.color_code_hint }),
      `${form.color_code_hint}. Must match the other radios or the repeater; 1 is common.`);
    if (form.slots.length) row('Time slot', select('slot', form.slots), 'Must match the other radios or the repeater.');
  } else {
    if (form.spacings.length) row('Channel spacing', [select('spacing', form.spacings), h('span', { class: 'unit' }, 'kHz')], '12.5 kHz is common today.');
    row('Tone (CTCSS)', [select('tone', ['', ...form.tones], true), h('span', { class: 'unit' }, 'Hz')],
      'Sent when transmitting; many repeaters only respond to their tone. Leave empty for none.');
    if (c.tone) row('Receive', check('tone_rx', 'Only open the speaker for signals with this tone'));
  }
  if (form.zone) row('Zone', check('add_to_zone', `Add it to zone "${form.zone}"`));
  // The select for an empty tone shows a blank entry; label it.
  for (const o of grid.querySelectorAll('option[value=""]')) o.textContent = 'None';
  return h('div', {}, choice, grid);
}

/** The starter channels and what replacing does, collapsed under the replace option. */
function renderStarter(form) {
  const q = state.quick;
  const key = 'channel/starter';
  const details = h('details', { class: 'quick-changes', open: q.openDetails.has(key) },
    h('summary', {}, 'Which channels'),
    h('table', { class: 'grid starter' },
      h('thead', {}, h('tr', {}, h('th', {}, 'Name'), h('th', {}, 'Type'), h('th', {}, 'Frequency'), h('th', {}, ''))),
      h('tbody', {}, form.starter.map((s) => h('tr', {},
        h('td', {}, s.name), h('td', {}, KIND_LABEL[s.kind]), h('td', {}, `${s.frequency} MHz`), h('td', { class: 'note' }, s.detail))))),
    h('p', { class: 'note' }, 'Existing channels are reset to CPS\'s channel template and reused in place, so the radio\'s current channel and scan lists stay valid; extra channels are removed. '
      + (form.zone ? `Zone "${form.zone}" then holds exactly these channels.` : '')
      + (form.other_zones ? ` Your other ${plural(form.other_zones, 'zone')} keep their channel numbers: check them under Zones afterwards.` : '')),
    h('p', { class: 'note' }, 'PMR446 frequencies are licence-free only for approved PMR446 radios (fixed antenna, 0.5 W). Check that you may use them with this radio where you are.'));
  details.addEventListener('toggle', () => { if (details.open) q.openDetails.add(key); else q.openDetails.delete(key); });
  return details;
}

function renderQuickReview() {
  const q = state.quick;
  const goBack = h('button', { onclick: () => quickGo(q.questions.length) }, 'Back');
  if (q.error) {
    const culprit = q.questions.findIndex((question) => q.error.startsWith(question.title));
    return h('section', { class: 'quick-card' },
      h('h2', {}, 'Review'),
      h('p', { class: 'quick-error' }, q.error),
      h('p', { class: 'note' }, 'Nothing was changed.'),
      h('div', { class: 'quick-buttons' }, goBack,
        culprit >= 0 ? h('button', { class: 'primary', onclick: () => quickGo(culprit + 1) }, `Fix "${q.questions[culprit].title}"`) : null));
  }
  const r = q.review;
  if (!r) return h('section', { class: 'quick-card' }, h('p', { class: 'note' }, 'Working…'));
  if (!r.changes.length) {
    return h('section', { class: 'quick-card' },
      h('h2', {}, 'Review'),
      h('p', {}, 'Nothing to change: every answer is either skipped or what the radio already has.'),
      h('div', { class: 'quick-buttons' }, goBack));
  }
  return h('section', { class: 'quick-card wide' },
    h('h2', {}, 'Review'),
    h('p', {}, `Your answers change ${plural(r.changes.length, 'setting')}:`),
    h('table', { class: 'grid changes' },
      h('thead', {}, h('tr', {}, h('th', {}, 'Where'), h('th', {}, 'Setting'), h('th', {}, 'Now'), h('th', {}, 'After'))),
      h('tbody', {}, r.changes.map((c) => h('tr', {}, h('td', {}, c.place), h('td', {}, c.setting), h('td', {}, c.old), h('td', { class: 'changed' }, c.new))))),
    ...r.notes.map((n) => h('p', { class: 'quick-note' }, n)),
    h('p', { class: 'note' }, 'They become pending changes like any edit: you can revert them one by one under Changes. Nothing is written to the radio until you use Write to radio….'),
    h('div', { class: 'quick-buttons' }, goBack, h('button', { class: 'primary', onclick: quickApply }, `Apply ${plural(r.changes.length, 'change')}`)));
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

async function onDuplicateChannel(kind, row) {
  if (!row) return;
  const result = await act('duplicate_channel', { kind, position: row.no - 1 }, `Added a copy of "${row.cells[0].text}"; rename it to tell them apart`);
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

// ---------- "More" menu: CSV and CPS templates ----------

/** The table shown, when the view is one (not settings or changes). */
function currentTableId() {
  return state.overview && state.table && state.view !== 'settings' && state.view !== 'changes' ? state.table.id : null;
}

function openMenu() {
  const table = currentTableId();
  for (const id of ['mi-csv-export', 'mi-csv-import']) {
    $(id).disabled = !table;
    $(id).title = table ? $(id).dataset.tip || $(id).title : 'Show a table (channels, zones, contacts or a list) first';
  }
  $('mi-csv-export').textContent = table ? `Export ${state.table.title} as CSV…` : 'Export table as CSV…';
  $('mi-csv-import').textContent = table ? `Import ${state.table.title} from CSV…` : 'Import table from CSV…';
  $('menu-more').hidden = false;
  $('btn-more').setAttribute('aria-expanded', 'true');
}

function closeMenu() {
  $('menu-more').hidden = true;
  $('btn-more').setAttribute('aria-expanded', 'false');
}

/** Run a menu entry: close the menu first, report unexpected errors. */
function menuAction(fn) {
  return async () => {
    closeMenu();
    try {
      await fn();
    } catch (e) {
      setStatus(errorText(e), 'error');
    }
  };
}

const plural = (n, word) => `${n} ${word}${n === 1 ? '' : 's'}`;

async function onExportCsv() {
  const table = currentTableId();
  if (!table) return;
  const defaultPath = await invoke('suggest_file', { kind: 'csv', table });
  const path = await pickSavePath({ title: `Export ${state.table.title} as CSV`, defaultPath, filters: [{ name: 'CSV (spreadsheet)', extensions: ['csv'] }] });
  if (path) await exportCsvTo(table, path);
}

async function exportCsvTo(table, path) {
  try {
    const n = await invoke('export_csv', { table, path });
    setStatus(`Exported ${plural(n, 'record')} of ${state.table?.title || table} to ${path}`, 'ok');
  } catch (e) {
    setStatus(errorText(e), 'error');
    await ask('Export failed', [h('pre', {}, errorText(e))], { cancel: false });
  }
}

async function onImportCsv() {
  const table = currentTableId();
  if (!table) return;
  const path = await pickPath({ title: `Import ${state.table.title} from CSV`, filters: [{ name: 'CSV (spreadsheet)', extensions: ['csv'] }] });
  if (path) await importCsvFrom(table, path);
}

async function importCsvFrom(table, path) {
  const title = state.table?.title || table;
  let result;
  try {
    result = await invoke('import_csv', { table, path });
  } catch (e) {
    // Cells accepted before a failure stay applied: show what is there now.
    state.overview = (await invoke('get_overview')) || state.overview;
    await refresh();
    setStatus(`CSV import failed: ${errorText(e)}`, 'error');
    await ask('CSV import failed', [h('pre', {}, errorText(e))], { cancel: false });
    return;
  }
  state.overview = result.overview;
  await refresh();
  const r = result.report;
  const done = r.changed || r.added
    ? `${plural(r.changed, 'value')} changed${r.added ? `, ${plural(r.added, 'record')} added` : ''}`
    : 'nothing to change (the file matches the table)';
  setStatus(`Imported ${path} into ${title}: ${done}${r.errors.length ? `; ${r.errors.length} refused` : ''}`, r.errors.length ? 'error' : 'ok');
  const body = [
    h('div', { class: 'kv' },
      h('span', {}, 'File'), h('span', {}, path),
      h('span', {}, 'Table'), h('span', {}, title),
      h('span', {}, 'Values changed'), h('span', {}, String(r.changed)),
      h('span', {}, 'Records added'), h('span', {}, String(r.added)),
      h('span', {}, 'Refused'), h('span', {}, String(r.errors.length))),
  ];
  if (r.errors.length) {
    body.push(h('p', {}, 'These were not applied (everything else was):'));
    body.push(h('ul', { class: 'refused' }, r.errors.map((e) => h('li', {}, e))));
  }
  if (r.changed || r.added) {
    body.push(h('p', { class: 'note' }, 'The changes are pending, like typed ones: review them under Changes, then use Save as… or Write to radio….'));
  }
  await ask('CSV import', body, { cancel: false });
}

async function onExportTemplate() {
  const d = await invoke('template_defaults');
  $('tpl-export-name').value = d.name;
  $('tpl-export-path').value = d.path;
  const n = state.overview.changes;
  $('tpl-export-changes').hidden = !n;
  $('tpl-export-changes').textContent = n ? `Includes your ${plural(n, 'unsaved change')}.` : '';
  if ((await showDialog($('dlg-tpl-export'))) !== 'go') return;
  await exportTemplateTo($('tpl-export-path').value.trim(), $('tpl-export-name').value.trim());
}

async function exportTemplateTo(path, name) {
  try {
    const r = await invoke('export_template', { path, name: name || null });
    setStatus(`Exported ${r.report.files} files as CPS template "${r.name}" to ${path}`, 'ok');
    await ask('Template exported', [
      h('div', { class: 'kv' },
        h('span', {}, 'Template name'), h('span', {}, r.name),
        h('span', {}, 'File'), h('span', {}, path),
        h('span', {}, 'Radio files'), h('span', {}, String(r.report.files))),
      ...r.report.warnings.map((w) => h('p', { class: 'note' }, `Note: ${w}`)),
      h('p', { class: 'note' }, 'In CPS, import it into the template library.'),
    ], { cancel: false });
  } catch (e) {
    setStatus(errorText(e), 'error');
    await ask('Export failed', [h('pre', {}, errorText(e))], { cancel: false });
  }
}

async function onImportTemplate() {
  if (!(await confirmDiscard())) return;
  const file = await pickPath({ title: 'CPS template to import', filters: [{ name: 'CPS template', extensions: ['tpl'] }] });
  if (!file) return;
  $('tpl-import-file').value = file;
  $('tpl-import-out').value = await invoke('suggest_folder', { kind: 'template' });
  if ((await showDialog($('dlg-tpl-import'))) !== 'go') return;
  await importTemplateFrom($('tpl-import-file').value, $('tpl-import-out').value.trim());
}

async function importTemplateFrom(file, out) {
  const reference = state.overview;
  let r;
  try {
    r = await invoke('import_template', { path: file, out });
  } catch (e) {
    setStatus(`Template import failed: ${errorText(e)}`, 'error');
    await ask('Template import failed', [h('pre', {}, errorText(e))], { cancel: false });
    return;
  }
  await opened(r.overview, `Imported CPS template "${r.name}" into ${out}`);
  const rep = r.report;
  const names = (list) => list.map(([id, name]) => `${id} ${name}`).join(', ');
  await ask('Template imported', [
    h('div', { class: 'kv' },
      h('span', {}, 'Template'), h('span', {}, `"${r.name}" (${file})`),
      h('span', {}, 'For radio'), h('span', {}, `${reference.model || '?'} · serial ${reference.serial || '?'} · RCDB ${reference.rcdb_version}`),
      h('span', {}, 'New folder'), h('span', {}, out),
      h('span', {}, 'Files from template'), h('span', {}, String(rep.from_template)),
      ...(rep.kept_from_reference.length ? [h('span', {}, 'Kept from open folder'), h('span', {}, names(rep.kept_from_reference))] : []),
      ...(rep.ignored.length ? [h('span', {}, 'Ignored (not on this radio)'), h('span', {}, names(rep.ignored))] : [])),
    ...(rep.warnings.length ? [h('p', {}, 'Warnings:'), h('ul', { class: 'refused' }, rep.warnings.map((w) => h('li', {}, w)))] : []),
    h('p', { class: 'note' }, 'The new folder is open. Nothing was written to the radio: check the settings, then use Write to radio….'),
  ], { cancel: false });
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
  for (const id of ['mi-csv-export', 'mi-csv-import']) $(id).dataset.tip = $(id).title;
  $('btn-more').addEventListener('click', (e) => {
    e.stopPropagation();
    if ($('menu-more').hidden) openMenu(); else closeMenu();
  });
  document.addEventListener('click', (e) => { if (!$('menu-more').hidden && !$('menu-more').contains(e.target)) closeMenu(); });
  document.addEventListener('keydown', (e) => { if (e.key === 'Escape') { closeMenu(); hideHelpPopover(); } });
  document.addEventListener('click', (e) => { if (!$('help-pop').hidden && !$('help-pop').contains(e.target)) hideHelpPopover(); });
  $('content').addEventListener('scroll', hideHelpPopover);
  window.addEventListener('resize', hideHelpPopover);
  $('mi-csv-export').addEventListener('click', menuAction(onExportCsv));
  $('mi-csv-import').addEventListener('click', menuAction(onImportCsv));
  $('mi-tpl-export').addEventListener('click', menuAction(onExportTemplate));
  $('mi-tpl-import').addEventListener('click', menuAction(onImportTemplate));
  loadHiddenPrefs();
  syncHiddenMenu();
  $('mi-show-licence').addEventListener('click', menuAction(() => setHiddenPref('licence', !state.showLicence)));
  $('mi-show-internal').addEventListener('click', menuAction(() => setHiddenPref('internal', !state.showInternal)));
  $('tpl-export-browse').addEventListener('click', async () => {
    const p = await pickSavePath({ title: 'Save the CPS template as', defaultPath: $('tpl-export-path').value, filters: [{ name: 'CPS template', extensions: ['tpl'] }] });
    if (p) $('tpl-export-path').value = p;
  });
  $('tpl-import-browse').addEventListener('click', async () => {
    const p = await pickPath({ title: 'CPS template to import', filters: [{ name: 'CPS template', extensions: ['tpl'] }] });
    if (p) $('tpl-import-file').value = p;
  });
  $('tpl-import-out-browse').addEventListener('click', () => browseNewFolder($('tpl-import-out')));
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
