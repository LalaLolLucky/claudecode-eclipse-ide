/* plugins.js — the "Manage Plugins" window (the / menu, or /plugins), laid out as the VS Code
   extension's is: a Plugins tab (what is installed, with its switch, update and remove, and what
   could be installed) and a Marketplaces tab (add, refresh, remove).

   Everything it does is the CLI's own `claude plugin …`, run in the tab's folder by the core
   (plugins.rs) and answered on the MCP window's channel, under a token the page chose: the list
   and the marketplaces, installing, updating, switching on and off and removing a plugin, adding
   and refreshing and removing a marketplace, and a plugin's options. Once a change is made the
   tab's own conversation is told to reload its plugins (reload_plugins), so it applies at once.

   The catalogue of the marketplaces is thousands of entries. It is read once, in the background
   after the installed plugins have been drawn, and drawn a page at a time. */

const PL_PAGE = 50;
/* Where an install goes, as `claude plugin install --scope` names it, worded as the extension words it. */
const PL_SCOPES = [
  ['user',    'Install for you',          'Available in all your projects'],
  ['project', 'Install for this project', 'Shared with all collaborators'],
  ['local',   'Install locally',          'Only for you, only in this repo'],
];
/* Shown with the choice of where to install, as the extension shows it. */
const PL_TRUST = 'Make sure you trust a plugin before installing, updating, or using it. Anthropic does not control what MCP '
  + 'servers, files, or other software are included in plugins and cannot verify that they will work as intended or that '
  + "they won't change.";
const PL_OFFICIAL = 'claude-plugins-official';
const PL_LONG_MS = 130000;   // a command that fetches may take two minutes (plugins.rs)
const PL_REFRESH = '<svg width="16" height="16" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.6" '
  + 'stroke-linecap="round" stroke-linejoin="round"><path d="M16.5 10a6.5 6.5 0 1 1-2-4.7"/><path d="M16.5 3.5v3.3h-3.3"/></svg>';

let plState = null;

/** One request of the window: `claude plugin …` in the tab's folder, resolved with its answer. */
function plCall(t, op, long) {
  if (!window._plugins) return Promise.reject(new Error('Not supported by this build.'));
  const token = mcpToken(), done = mcpAwait(token, long ? PL_LONG_MS : 0);
  _plugins(t.id, token, JSON.stringify(op), rootPathOf(t));
  return done;
}

function openPlugins() {
  closeMenus(); closeSlash();
  const t = activeTab(); if (!t) return;
  if (plState) closePlugins(true);
  plState = {
    tab: t, view: 'plugins', installed: [], available: null, availLoading: true, availError: null,
    marketplaces: [], filter: '', shown: PL_PAGE, success: null, failure: null,
    busy: new Set(), scopeFor: null, confirm: null, addText: '', adding: false, options: null,
  };
  document.getElementById('pl-win').dataset.view = '';
  document.getElementById('pl-overlay').classList.add('open');
  document.addEventListener('keydown', plKey, true);
  registerOverlayCancel(plCancel, false);
  renderPlugins();
  plLoad();
}
function closePlugins(force) {
  const st = plState; if (!st) return true;
  if (!force && st.adding) return false;
  plCloseDialog();
  plState = null;
  document.getElementById('pl-win').dataset.view = '';
  document.getElementById('pl-overlay').classList.remove('open');
  document.removeEventListener('keydown', plKey, true);
  unregisterOverlayCancel(plCancel);
  return true;
}
/** The dismiss key: backs out of the confirmation dialog, the options form or a pending confirmation first. */
function plEscape() {
  const st = plState; if (!st) return;
  if (plCloseDialog()) return;
  if (st.view === 'options') { if (!st.options.saving) { st.view = 'plugins'; renderPlugins(); } return; }
  if (st.confirm) { st.confirm = null; plFill(); return; }
  if (st.scopeFor) { st.scopeFor = null; plFill(); return; }
  closePlugins(false);
}
/* Eclipse's binding (Esc, or Ctrl+G under Emacs) pops this entry before calling it, so a window
   still open afterwards has to put it back. */
function plCancel() {
  plEscape();
  if (plState) registerOverlayCancel(plCancel, false);
}
document.getElementById('pl-overlay').addEventListener('click', (e) => {
  if (e.target.id === 'pl-overlay') closePlugins(false);
});
function plKey(e) {
  if (e.key !== 'Escape' || !plState) return;
  e.preventDefault(); e.stopPropagation();
  plEscape();
}

/* ---- loading ---- */
function plMessage(st, ok, text) { st.success = ok ? text : null; st.failure = ok ? null : text; }
function plErr(err, fallback) { return (err && err.message) || fallback; }

/** The installed plugins and the marketplaces first, which are quick; then the catalogue. */
function plLoad() {
  const st = plState; if (!st) return;
  plRefreshInstalled().then(() => plRefreshMarketplaces());
  plRefreshAvailable();
}
function plRefreshInstalled() {
  const st = plState; if (!st) return Promise.resolve();
  return plCall(st.tab, { op: 'list' }).then((res) => {
    if (plState !== st) return;
    st.installed = res.installed || [];
  }, (err) => { if (plState === st) plMessage(st, false, plErr(err, 'Failed to load plugins')); })
    .then(() => { if (plState === st) plFill(); });
}
function plRefreshMarketplaces() {
  const st = plState; if (!st) return Promise.resolve();
  return plCall(st.tab, { op: 'marketplaces' }).then((res) => {
    if (plState !== st) return;
    st.marketplaces = res.marketplaces || [];
  }, (err) => { if (plState === st) plMessage(st, false, plErr(err, 'Failed to load marketplaces')); })
    .then(() => { if (plState === st) plFill(); });
}
function plRefreshAvailable() {
  const st = plState; if (!st) return Promise.resolve();
  st.availLoading = true; st.availError = null;
  return plCall(st.tab, { op: 'list', available: true }, true).then((res) => {
    if (plState !== st) return;
    st.available = res.available || [];
    st.installed = res.installed || st.installed;
  }, (err) => { if (plState === st) st.availError = plErr(err, 'Failed to load available plugins'); })
    .then(() => { if (plState === st) { st.availLoading = false; plFill(); } });
}

/* ---- actions ---- */
const PL_DONE = { install: 'Installed', update: 'Updated', enable: 'Enabled', disable: 'Disabled', uninstall: 'Uninstalled' };
/** Runs one change to a plugin, then has the conversation reload its plugins and re-reads the
    list. `key` marks what is busy. */
function plChange(op, label, key) {
  const st = plState; if (!st || plBusy(st)) return Promise.resolve();
  st.busy.add(key); st.success = null; st.failure = null; plFill();
  return plCall(st.tab, op, op.op === 'install' || op.op === 'update').then(() => plApply(st, PL_DONE[op.op] + ' ' + label), (err) => {
    if (plState === st) plMessage(st, false, plErr(err, 'Failed to ' + op.op + ' ' + label));
  }).then((result) => {
    if (plState !== st) return;
    return plRefreshInstalled().then(() => {
      if (plState !== st) return;
      st.busy.delete(key); plFill();
      return result;
    });
  });
}
/** Tells the tab's conversation to reload its plugins and words the outcome. */
function plApply(st, done) {
  return mcpCall(st.tab, { subtype: 'reload_plugins' }).then(
    () => { if (plState === st) plMessage(st, true, done + '. Applied to this conversation.'); return true; },
    () => { if (plState === st) plMessage(st, true, done + '. Restart the conversation to apply it.'); return true; });
}
function plInstall(p, scope) {
  const st = plState; if (!st || plBusy(st)) return;
  st.scopeFor = null;
  plChange({ op: 'install', id: p.pluginId, scope }, p.name, p.pluginId + ':install').then((ok) => {
    if (ok && plState === st) plOptionsAfterInstall(p);
  });
}
function plToggle(p) {
  plChange({ op: p.enabled ? 'disable' : 'enable', id: p.id, scope: p.scope }, plName(p.id), p.id + ':toggle');
}
function plUpdate(p) { plChange({ op: 'update', id: p.id, scope: p.scope }, plName(p.id), p.id + ':update'); }
function plUninstall(p) {
  const st = plState; if (!st) return;
  st.confirm = null;
  plChange({ op: 'uninstall', id: p.id, scope: p.scope }, plName(p.id), p.id + ':uninstall');
}
function plMarketplaceChange(op, doneText, failText, key) {
  const st = plState; if (!st || plBusy(st)) return Promise.resolve();
  st.busy.add(key); st.success = null; st.failure = null; plFill();
  return plCall(st.tab, op, true).then(() => { if (plState === st) plMessage(st, true, doneText); }, (err) => {
    if (plState === st) plMessage(st, false, plErr(err, failText));
  }).then(() => {
    if (plState !== st) return;
    return plRefreshMarketplaces().then(() => {
      if (plState !== st) return;
      st.busy.delete(key); plFill();
      plRefreshAvailable();
    });
  });
}
function plAddMarketplace() {
  const st = plState; if (!st || st.adding) return;
  const source = st.addText.trim();
  if (!source) { plMessage(st, false, 'Enter a GitHub repo, URL, or path.'); plFill(); return; }
  st.adding = true; st.success = null; st.failure = null; plFill();
  plCall(st.tab, { op: 'marketplaceAdd', source }, true).then(() => {
    if (plState !== st) return;
    st.addText = ''; plMessage(st, true, 'Added marketplace ' + source + '.');
  }, (err) => { if (plState === st) plMessage(st, false, plErr(err, 'Failed to add marketplace')); }).then(() => {
    if (plState !== st) return;
    st.adding = false; plFill();
    plRefreshMarketplaces().then(() => plRefreshAvailable());
  });
}

/* ---- options ---- */
/** The values the form starts from, and what it can tell apart. `keys` are the options shown. */
function plOpenOptions(p, name, keys, state) {
  const st = plState; if (!st) return;
  const values = {};
  keys.forEach(k => {
    const choices = state.choices && state.choices[k];
    const given = state.inputs && state.inputs[k];
    // A drop-down shows its first choice when nothing was chosen, so that is the value.
    values[k] = given != null ? String(given) : (Array.isArray(choices) && choices.length ? String(choices[0]) : '');
  });
  st.options = { pluginId: p, name, keys, state, values, initial: Object.assign({}, values), touched: new Set(),
                 saving: false, error: null, fieldError: null };
  st.view = 'options';
  renderPlugins();
}
/** What a plugin lets the reader set: asked of the CLI, then shown if there is any. */
function plConfigure(id) {
  const st = plState; if (!st) return;
  const key = id + ':options';
  if (plBusy(st)) return;
  st.busy.add(key); st.success = null; st.failure = null; plFill();
  plCall(st.tab, { op: 'options', id }, true).then((o) => {
    if (plState !== st) return;
    const keys = Object.keys(o.schema || {});
    if (!keys.length) plMessage(st, true, plName(id) + ' has no options.');
    else plOpenOptions(id, plName(id), keys, o);
  }, (err) => { if (plState === st) plMessage(st, false, plErr(err, 'Failed to read the options of ' + plName(id))); })
    .then(() => { if (plState === st) { st.busy.delete(key); if (st.view === 'plugins') plFill(); } });
}
/** After an install: the options it left unset, if it has any. */
function plOptionsAfterInstall(p) {
  const st = plState; if (!st) return;
  plCall(st.tab, { op: 'options', id: p.pluginId }, true).then((o) => {
    if (plState !== st) return;
    const keys = (o.unconfigured || []).filter(k => Object.prototype.hasOwnProperty.call(o.schema || {}, k));
    if (keys.length) plOpenOptions(p.pluginId, p.name, keys, o);
  }, () => {});
}
function plOptionRequired(o, k) {
  const sc = (o.state.schema || {})[k];
  const unchangedSecret = (o.state.configured || []).includes(k) && (o.state.choices || {})[k] === undefined
    && (o.initial[k] || '') === '';
  return !!(sc && sc.required === true) && !unchangedSecret;
}
function plSaveOptions() {
  const st = plState, o = st && st.options; if (!o || o.saving) return;
  if (o.keys.some(k => plOptionRequired(o, k) && (o.values[k] || '').trim() === '')) return;
  const values = {};
  o.keys.forEach(k => {
    const choice = (o.state.choices || {})[k] !== undefined && (o.state.unconfigured || []).includes(k);
    if (o.touched.has(k) || choice) values[k] = o.values[k] || '';
  });
  o.saving = true; o.error = null; renderPlugins();
  plCall(st.tab, { op: 'saveOptions', id: o.pluginId, values }, true).then(() => {
    if (plState !== st) return;
    st.view = 'plugins'; st.options = null;
    return plApply(st, 'Saved the options of ' + o.name);
  }, (err) => {
    if (plState !== st) return;
    o.saving = false; o.error = plErr(err, 'Something went wrong while saving.'); renderPlugins();
  }).then(() => { if (plState === st) renderPlugins(); });
}

/* ---- rendering ---- */
function plEl(tag, cls, text) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text != null) e.textContent = text;
  return e;
}
/** Whether something is being done: a change, a reload after it, a marketplace being added. While it is, every button waits. */
function plBusy(st) { return st.busy.size > 0 || st.adding; }
function plName(id) { return String(id).split('@')[0]; }
function plCount(n) { return n >= 1e6 ? (n / 1e6).toFixed(1).replace(/\.0$/, '') + 'm' : n >= 1e3 ? (n / 1e3).toFixed(1).replace(/\.0$/, '') + 'k' : String(n); }
function plButton(label, onClick, opts) {
  opts = opts || {};
  const b = plEl('button', 'pl-btn' + (opts.primary ? ' primary' : '') + (opts.danger ? ' danger' : ''), label);
  b.type = 'button'; b.disabled = !!opts.disabled; b.onclick = onClick;
  return b;
}
function plIconButton(html, title, onClick, disabled) {
  const b = plEl('button', 'pl-icon'); b.type = 'button'; b.innerHTML = html; b.title = title;
  b.disabled = !!disabled; b.onclick = onClick;
  return b;
}
function plMessages(parent, st) {
  if (st.success) parent.appendChild(plEl('div', 'pl-msg ok', st.success));
  if (st.failure) parent.appendChild(plEl('div', 'pl-msg err', st.failure));
}
function plLink(text, href) {
  const a = plEl('a', 'pl-link', text); a.href = href; return a;
}
/** The link a plugin's source stands for. A source inside its own marketplace's repository is
    named by a relative path, which the marketplace's GitHub repo turns into a page. */
function plSourceLink(p, st) {
  const s = p.source;
  const mk = st.marketplaces.find(m => m.name === p.marketplace);
  const tree = (base, ref, path) => base + (path ? '/tree/' + (ref || 'main') + '/' + String(path).replace(/^\.\//, '') : '');
  const trim = (u) => String(u).replace(/\.git$/, '');
  if (typeof s === 'string') {
    if (mk && mk.source === 'github' && mk.repo) return tree('https://github.com/' + mk.repo, null, s);
    return null;
  }
  if (!s) return null;
  if (s.source === 'github' && s.repo) return tree('https://github.com/' + s.repo, s.ref, s.path);
  if (s.url && /^https?:/i.test(s.url)) return tree(trim(s.url), s.ref, s.path);
  return null;
}
function plMarketplaceSource(m) {
  if (m.source === 'github' && m.repo) return { label: 'GitHub: ' + m.repo, href: 'https://github.com/' + m.repo };
  if ((m.source === 'git' || m.source === 'url') && m.url) {
    return { label: m.url, href: /^https?:/i.test(m.url) ? m.url.replace(/\.git$/, '') : null };
  }
  if (m.path) return { label: (m.source === 'file' ? 'File: ' : 'Directory: ') + m.path, href: null };
  if (m.package) return { label: 'npm: ' + m.package, href: null };
  return { label: m.source || '', href: null };
}

function renderPlugins() {
  const st = plState; if (!st) return;
  const win = document.getElementById('pl-win');
  if (win.dataset.view === st.view && st.view !== 'options') { plFill(); return; }
  win.innerHTML = '';
  win.dataset.view = st.view;
  const head = plEl('div', 'pl-head');
  head.appendChild(plEl('span', 'pl-title', 'Manage Plugins'));
  const x = plEl('span', 'pl-x'); x.innerHTML = ICONS.X; x.title = 'Close';
  x.onclick = () => closePlugins(false);
  head.appendChild(x);
  win.appendChild(head);
  if (st.view === 'options') { plBuildOptions(win, st); return; }
  const tabs = plEl('div', 'pl-tabs'); tabs.id = 'pl-tabs';
  win.appendChild(tabs);
  const body = plEl('div', 'pl-body'); body.id = 'pl-body';
  win.appendChild(body);
  plFill();
}
/** Draws what the window holds into the skeleton it has. The search box and the add box are kept
    where they stand, so typing in one never loses the caret to a repaint. */
function plFill() {
  const st = plState; if (!st) return;
  if (st.view === 'options') return;
  const tabs = document.getElementById('pl-tabs'), body = document.getElementById('pl-body');
  if (!tabs || !body) { renderPlugins(); return; }
  tabs.innerHTML = '';
  [['plugins', 'Plugins', st.installed.length], ['marketplaces', 'Marketplaces', st.marketplaces.length]].forEach(([id, label, n]) => {
    const tab = plEl('button', 'pl-tab' + (st.view === id ? ' active' : ''));
    tab.type = 'button'; tab.appendChild(document.createTextNode(label));
    tab.appendChild(plEl('span', 'pl-count', String(n)));
    tab.onclick = () => { if (st.view !== id) { st.view = id; st.confirm = null; st.scopeFor = null; st.success = null; st.failure = null; document.getElementById('pl-win').dataset.view = ''; renderPlugins(); } };
    tabs.appendChild(tab);
  });
  const keepFilter = st.view === 'plugins' && body.querySelector('.pl-search');
  const keepAdd = st.view === 'marketplaces' && body.querySelector('.pl-add-input');
  const old = keepFilter || keepAdd;
  if (old) old.remove();
  body.innerHTML = '';
  if (st.view === 'plugins') plBuildPlugins(body, st, keepFilter);
  else plBuildMarketplaces(body, st, keepAdd);
}

function plBuildPlugins(body, st, kept) {
  let search = kept;
  if (!search) {
    search = plEl('input', 'pl-input pl-search'); search.type = 'text'; search.placeholder = 'Search plugins…';
    search.value = st.filter; search.autocomplete = 'off';
    search.oninput = () => { st.filter = search.value; st.shown = PL_PAGE; plFillList(); };
  }
  body.appendChild(search);
  const list = plEl('div', 'pl-list'); list.id = 'pl-list';
  body.appendChild(list);
  plFillList();
  if (!kept) setTimeout(() => search.focus(), 0);
}
function plMatches(q, ...fields) { return !q || fields.some(f => String(f || '').toLowerCase().includes(q)); }
/** The messages, the installed plugins and the available ones, for what the search box says. */
function plFillList() {
  const st = plState; const list = document.getElementById('pl-list'); if (!st || !list) return;
  list.innerHTML = '';
  plMessages(list, st);
  const q = st.filter.trim().toLowerCase();
  const installed = st.installed.filter(p => plMatches(q, p.id, ...(p.mcpServers || [])));
  list.appendChild(plEl('div', 'pl-section', 'Installed'));
  if (!installed.length) list.appendChild(plEl('div', 'pl-empty', q ? 'No installed plugins match.' : 'No plugins installed.'));
  installed.forEach(p => list.appendChild(plInstalledRow(p, st)));

  list.appendChild(plEl('div', 'pl-section', 'Available'));
  if (st.availLoading) list.appendChild(plEl('div', 'pl-empty', 'Loading available plugins…'));
  else if (st.availError) {
    const e = plEl('div', 'pl-empty'); e.appendChild(plEl('span', null, st.availError + ' '));
    const retry = plEl('a', 'pl-link', 'Try again'); retry.href = '#';
    retry.onclick = (ev) => { ev.preventDefault(); plRefreshAvailable().then(() => plFill()); plFill(); };
    e.appendChild(retry); list.appendChild(e);
  } else {
    const have = new Set(st.installed.map(p => p.id));
    const hits = (st.available || []).filter(p => !have.has(p.pluginId) && plMatches(q, p.name, p.description, p.marketplace));
    if (!hits.length) list.appendChild(plEl('div', 'pl-empty', q ? 'No available plugins match.' : 'Nothing to install. Add a marketplace to find plugins.'));
    hits.slice(0, st.shown).forEach(p => list.appendChild(plAvailableRow(p, st)));
    if (hits.length > st.shown) {
      list.appendChild(plButton('Show more (' + (hits.length - st.shown) + ')', () => { st.shown += PL_PAGE; plFillList(); }));
    }
  }
}
function plInstalledRow(p, st) {
  const row = plEl('div', 'pl-row');
  const main = plEl('div', 'pl-main');
  const title = plEl('div', 'pl-name');
  title.appendChild(plEl('span', 'pl-dot' + (p.enabled ? ' on' : '')));
  title.appendChild(document.createTextNode(p.id));
  main.appendChild(title);
  main.appendChild(plEl('div', 'pl-sub', 'Scope: ' + p.scope));
  if ((p.mcpServers || []).length) {
    const chips = plEl('div', 'pl-chips');
    p.mcpServers.forEach(n => { const c = plEl('span', 'pl-chip'); c.appendChild(plEl('span', 'pl-dot on')); c.appendChild(document.createTextNode(n)); chips.appendChild(c); });
    main.appendChild(chips);
  }
  row.appendChild(main);
  const acts = plEl('div', 'pl-acts');
  const busy = (k) => st.busy.has(p.id + ':' + k);
  const own = ['toggle', 'update', 'uninstall', 'options'].some(busy);
  const any = plBusy(st);   // one thing at a time: the rest wait
  if (st.confirm && st.confirm.id === p.id) {
    acts.appendChild(plEl('span', 'pl-confirm-text', 'Uninstall ' + plName(p.id) + '?'));
    acts.appendChild(plButton('Uninstall', () => plUninstall(p), { danger: true }));
    acts.appendChild(plButton('Cancel', () => { st.confirm = null; plFillList(); }));
  } else {
    const sw = plEl('button', 'pl-switch' + (p.enabled ? ' on' : '')); sw.type = 'button';
    sw.title = p.enabled ? 'Disable' : 'Enable'; sw.disabled = any; sw.onclick = () => plToggle(p);
    sw.setAttribute('role', 'switch'); sw.setAttribute('aria-checked', String(p.enabled));
    sw.appendChild(plEl('span', 'pl-knob'));
    acts.appendChild(sw);
    acts.appendChild(plIconButton(ICONS.GEAR, 'Options', () => plConfigure(p.id), any));
    acts.appendChild(plIconButton(PL_REFRESH, 'Update', () => plUpdate(p), any));
    acts.appendChild(plIconButton(ICONS.TRASH, 'Uninstall', () => { st.confirm = { id: p.id }; plFillList(); }, any));
  }
  row.appendChild(acts);
  if (own) row.classList.add('busy');
  return row;
}
function plAvailableRow(p, st) {
  const row = plEl('div', 'pl-row avail');
  const top = plEl('div', 'pl-top');
  top.appendChild(plEl('span', 'pl-name', p.name));
  const side = plEl('div', 'pl-acts');
  if (p.installCount) side.appendChild(plEl('span', 'pl-installs', plCount(p.installCount) + ' installs'));
  const key = p.pluginId + ':install';
  if (st.scopeFor !== p.pluginId) {
    side.appendChild(plButton(st.busy.has(key) ? 'Installing…' : 'Install', () => {
      st.scopeFor = p.pluginId; plFillList();
      // The choices are below the row, which may be at the foot of the list: bring them into view.
      const pick = document.querySelector('#pl-list .pl-scopes');
      if (pick) pick.scrollIntoView({ block: 'nearest' });
    }, { primary: true, disabled: plBusy(st) }));
  }
  top.appendChild(side);
  row.appendChild(top);
  if (p.description) row.appendChild(plEl('div', 'pl-desc', p.description));
  const from = plEl('div', 'pl-sub');
  from.appendChild(document.createTextNode('from ' + p.marketplace));
  if (p.marketplace === PL_OFFICIAL) { const m = plEl('span', 'pl-official'); m.innerHTML = ICONS.SUNBURST; from.appendChild(m); }
  row.appendChild(from);
  const href = plSourceLink(p, st);
  if (href) { const s = plEl('div', 'pl-sub'); s.appendChild(document.createTextNode('Source: ')); s.appendChild(plLink(href, href)); row.appendChild(s); }
  if (st.scopeFor === p.pluginId) {
    const pick = plEl('div', 'pl-scopes');
    pick.appendChild(plEl('div', 'pl-trust', PL_TRUST));
    PL_SCOPES.forEach(([id, label, hint]) => {
      const b = plEl('button', 'pl-scope'); b.type = 'button';
      b.appendChild(plEl('span', 'pl-scope-name', label)); b.appendChild(plEl('span', 'pl-scope-hint', hint));
      b.onclick = () => plInstall(p, id); b.disabled = plBusy(st);
      pick.appendChild(b);
    });
    const cancel = plEl('a', 'pl-cancel', 'Cancel'); cancel.href = '#';
    cancel.onclick = (e) => { e.preventDefault(); st.scopeFor = null; plFillList(); };
    pick.appendChild(cancel);
    row.appendChild(pick);
  }
  return row;
}

function plBuildMarketplaces(body, st, kept) {
  const add = plEl('div', 'pl-add');
  let input = kept;
  if (!input) {
    input = plEl('input', 'pl-input pl-add-input'); input.type = 'text'; input.autocomplete = 'off';
    input.placeholder = 'GitHub repo, URL, or path...'; input.value = st.addText;
    input.oninput = () => { st.addText = input.value; const b = document.getElementById('pl-add-btn'); if (b) b.disabled = plBusy(st) || !input.value.trim(); };
    input.onkeydown = (e) => { if (e.key === 'Enter' && !e.isComposing) { e.preventDefault(); plAddMarketplace(); } };
  }
  input.value = st.addText; input.disabled = st.adding;
  add.appendChild(input);
  const btn = plButton(st.adding ? 'Adding…' : 'Add', plAddMarketplace, { primary: true, disabled: plBusy(st) || !st.addText.trim() });
  btn.id = 'pl-add-btn';
  add.appendChild(btn);
  body.appendChild(add);
  const list = plEl('div', 'pl-list');
  plMessages(list, st);
  if (!st.marketplaces.length) list.appendChild(plEl('div', 'pl-empty', 'No marketplaces yet.'));
  st.marketplaces.forEach(m => list.appendChild(plMarketplaceRow(m, st)));
  body.appendChild(list);
  if (!kept) setTimeout(() => input.focus(), 0);
}
function plMarketplaceRow(m, st) {
  const row = plEl('div', 'pl-row');
  const main = plEl('div', 'pl-main');
  const title = plEl('div', 'pl-name');
  title.appendChild(document.createTextNode(m.name));
  if (m.name === PL_OFFICIAL) { const o = plEl('span', 'pl-official'); o.innerHTML = ICONS.SUNBURST; title.appendChild(o); }
  main.appendChild(title);
  const src = plMarketplaceSource(m);
  main.appendChild(src.href ? plLink(src.label, src.href) : plEl('div', 'pl-sub', src.label));
  row.appendChild(main);
  const acts = plEl('div', 'pl-acts');
  const own = st.busy.has(m.name + ':refresh') || st.busy.has(m.name + ':remove');
  const waiting = plBusy(st);
  {
    acts.appendChild(plIconButton(PL_REFRESH, 'Update from its source', () =>
      plMarketplaceChange({ op: 'marketplaceUpdate', name: m.name }, 'Updated marketplace ' + m.name + '.', 'Failed to update ' + m.name, m.name + ':refresh'),
      waiting));
    acts.appendChild(plIconButton(ICONS.TRASH, 'Remove', () => plConfirmRemoveMarketplace(m), waiting));
  }
  if (own) row.classList.add('busy');
  row.appendChild(acts);
  return row;
}

/* ---- the confirmation dialog ----
   A small dialog over the window, as the extension's: a title, a message, the thing it is about in
   a line of its own, then a button for each choice, stacked, with the safe one (Cancel) the
   highlighted one. Escape cancels; a click outside does not. */
function plDialog(spec) {
  const st = plState; if (!st) return;
  plCloseDialog();
  const scrim = plEl('div', 'pl-dialog-scrim'); scrim.id = 'pl-dialog';
  const box = plEl('div', 'pl-dialog'); box.setAttribute('role', 'alertdialog');
  box.appendChild(plEl('div', 'pl-dialog-title', spec.title));
  box.appendChild(plEl('p', 'pl-dialog-msg', spec.message));
  if (spec.detail) box.appendChild(plEl('p', 'pl-dialog-detail', spec.detail));
  const btns = plEl('div', 'pl-dialog-btns');
  let cancel;
  spec.buttons.concat([{ label: 'Cancel', primary: true }]).forEach(b => {
    const btn = plButton(b.label, () => { plCloseDialog(); if (b.onClick) b.onClick(); }, { primary: !!b.primary });
    if (b.label === 'Cancel') cancel = btn;
    btns.appendChild(btn);
  });
  box.appendChild(btns); scrim.appendChild(box);
  document.getElementById('pl-overlay').appendChild(scrim);
  cancel.focus();
}
function plCloseDialog() {
  const d = document.getElementById('pl-dialog');
  if (!d) return false;
  d.remove();
  return true;
}
/** What removing a marketplace does besides, which is more than the name says: the extension says so. */
function plConfirmRemoveMarketplace(m) {
  const st = plState; if (!st) return;
  const names = st.installed.filter(p => p.id.endsWith('@' + m.name)).map(p => '"' + plName(p.id) + '"');
  const list = names.length > 1 ? names.slice(0, -1).join(', ') + ' and ' + names[names.length - 1] : (names[0] || '');
  const n = names.length;
  plDialog({
    title: 'Remove this marketplace?',
    message: n === 0 ? 'None of its plugins are installed.'
      : 'This usually also uninstalls ' + (n === 1 ? 'its installed plugin' : 'its ' + n + ' installed plugins') + ', ' + list
        + ', and deletes what ' + (n === 1 ? 'it' : 'they') + ' saved.',
    detail: m.name,
    buttons: [{ label: 'Remove', onClick: () => plMarketplaceChange({ op: 'marketplaceRemove', name: m.name },
      'Removed marketplace ' + m.name + '.', 'Failed to remove ' + m.name, m.name + ':remove') }],
  });
}

/** The options form, as the extension draws it: one field per option, a drop-down where the plugin
    offers choices, a password box for a secret, a star on what is required. */
function plBuildOptions(win, st) {
  const o = st.options;
  const body = plEl('div', 'pl-body');
  const back = plEl('a', 'pl-back', '← Back to list'); back.href = '#';
  back.onclick = (e) => { e.preventDefault(); if (!o.saving) { st.view = 'plugins'; renderPlugins(); } };
  body.appendChild(back);
  body.appendChild(plEl('div', 'pl-form-title', o.name));
  const save = plButton(o.saving ? 'Saving…' : 'Save configuration', () => plSaveOptions(), { primary: true });
  const sync = () => { save.disabled = o.saving || o.keys.some(k => plOptionRequired(o, k) && (o.values[k] || '').trim() === ''); };
  o.keys.forEach((k, i) => {
    const sc = (o.state.schema || {})[k] || {}, choices = (o.state.choices || {})[k];
    const field = plEl('div', 'pl-field');
    const label = plEl('label', 'pl-label', sc.title || k); label.htmlFor = 'pl-opt-' + i;
    if (plOptionRequired(o, k)) label.appendChild(plEl('span', 'pl-star', '*'));
    field.appendChild(label);
    let input;
    if (Array.isArray(choices)) {
      input = plEl('select', 'pl-input');
      choices.forEach(c => { const op = plEl('option', null, String(c)); op.value = String(c); input.appendChild(op); });
      input.value = o.values[k];
    } else {
      input = plEl('input', 'pl-input'); input.type = sc.sensitive === true ? 'password' : 'text'; input.autocomplete = 'off';
      input.value = o.values[k];
      if ((o.state.configured || []).includes(k) && (o.initial[k] || '') === '') input.placeholder = '(unchanged)';
      input.onkeydown = (e) => { if (e.key === 'Enter' && !e.isComposing) { e.preventDefault(); plSaveOptions(); } };
    }
    input.id = 'pl-opt-' + i; input.disabled = o.saving;
    const change = () => { o.values[k] = input.value; o.touched.add(k); sync(); };
    input.oninput = change; input.onchange = change;
    field.appendChild(input);
    if (sc.description) field.appendChild(plEl('div', 'pl-sub', sc.description));
    body.appendChild(field);
  });
  sync();
  const actions = plEl('div', 'pl-form-actions'); actions.appendChild(save);
  body.appendChild(actions);
  if (o.error) body.appendChild(plEl('div', 'pl-msg err', o.error));
  win.appendChild(body);
  const first = body.querySelector('input, select');
  if (first && !o.saving) setTimeout(() => first.focus(), 0);
}
