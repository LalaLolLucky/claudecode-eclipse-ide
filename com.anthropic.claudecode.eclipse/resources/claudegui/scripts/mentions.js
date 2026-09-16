/* mentions.js — the composer's @ list and + menu, and @-mentions in sent messages,
   following the VS Code extension (joebiden, joebiden2).

   Typing "@" lists every file and folder under the conversation's working folder —
   the directory tab it belongs to, not necessarily the workspace root — narrowing as
   the word grows. "@browser:" lists Chrome tabs instead. Picking a row writes the
   mention into the message; the CLI reads the file itself when the message is sent. */

const mentionEl = document.getElementById('mention-menu');
const mentionState = { open: false, sel: 0, items: [], query: null, reqSeq: 0, pendingId: '', timer: 0 };

/** The "@word" the caret is in, as {query, start, end} — or null. */
function mentionAtCaret() {
  if (input.selectionStart !== input.selectionEnd) return null;
  const v = input.value, caret = input.selectionStart;
  const re = /(?:^|\s)@[^\s]*/gm;
  let m;
  while ((m = re.exec(v))) {
    const start = v.indexOf('@', m.index), end = m.index + m[0].length;
    if (caret >= start && caret <= end) return { query: v.substring(start + 1, end), start, end };
  }
  return null;
}

/* Re-reads the word under the caret after every edit or caret move, and asks for the
   rows 200 ms after the last change, as the extension does. */
function updateMentionMenu() {
  const at = mentionAtCaret();
  if (!at) { closeMention(); return; }
  if (at.query === mentionState.query && (mentionState.open || mentionState.timer)) return;
  mentionState.query = at.query;
  clearTimeout(mentionState.timer);
  mentionState.timer = setTimeout(() => {
    mentionState.timer = 0;
    const id = String(++mentionState.reqSeq);
    mentionState.pendingId = id;
    if (window._listFilesAsync) window._listFilesAsync(id, rootPathOf(activeTab()) || '', at.query);
  }, 200);
}

window.onFilesListed = function (id) {
  if (id !== mentionState.pendingId || !window._takeFilesListed) return;
  let rows = [];
  try { rows = JSON.parse(window._takeFilesListed(id) || '[]'); } catch (e) { rows = []; }
  const at = mentionAtCaret();
  if (!at || at.query !== mentionState.query) return;
  mentionState.items = Array.isArray(rows) ? rows : [];
  mentionState.sel = 0;
  mentionState.open = true;
  renderMention();
  mentionEl.classList.add('open');
  renderBrowserBanner();   // the list takes the banner's place while it is open
};

function closeMention() {
  clearTimeout(mentionState.timer);
  mentionState.timer = 0;
  const wasOpen = mentionState.open;
  mentionState.open = false;
  mentionState.query = null;
  mentionState.pendingId = '';
  mentionEl.classList.remove('open');
  if (wasOpen) renderBrowserBanner();
}

/* ---- the browser banner ----
   Per conversation, like the extension's: "Browser connected" once Chrome is added to
   the conversation's process, and its × takes it back out. The state comes from the
   core as {status:"connecting"|"connected"|"error"|"disconnected", error?}. */
window.onBrowserState = function (tabId, json) {
  const t = tabs.find(x => x.id === tabId);
  if (!t) return;
  let d = null;
  try { d = JSON.parse(json || '{}'); } catch (e) { return; }
  t.browserState = d && d.status && d.status !== 'disconnected' ? d : null;
  if (t === activeTab()) renderBrowserBanner();
};

function browserBannerText(s) {
  switch (s.status) {
    case 'connecting': return 'Connecting to browser…';
    case 'connected': return 'Browser connected';
    case 'error': return 'Browser error: ' + (s.error || '');
    default: return '';
  }
}

/** Shows the active conversation's browser state, or hides the banner. */
function renderBrowserBanner() {
  const el = document.getElementById('browser-banner');
  if (!el) return;
  const t = activeTab();
  const s = t && t.browserState;
  const text = s ? browserBannerText(s) : '';
  if (!text || mentionState.open) { el.hidden = true; return; }
  el.querySelector('.bb-txt').textContent = text;
  el.dataset.color = s.status === 'error' ? 'error' : 'normal';
  el.hidden = false;
}

function disconnectBrowser() {
  const t = activeTab();
  if (!t) return;
  t.browserState = null;
  renderBrowserBanner();
  if (window._disconnectBrowser) window._disconnectBrowser(t.id);
}

function renderMention() {
  const list = document.createElement('div');
  list.className = 'mention-list';
  if (!mentionState.items.length) {
    const empty = document.createElement('div');
    empty.className = 'mention-empty';
    empty.textContent = String(mentionState.query || '').startsWith('browser:') ? 'No browser tabs found' : 'No files found';
    list.appendChild(empty);
  }
  mentionState.items.forEach((it, i) => {
    const row = document.createElement('div');
    row.className = 'mention-item' + (i === mentionState.sel ? ' sel' : '');
    row.onmousemove = () => { if (mentionState.sel !== i) { mentionState.sel = i; paintMentionSel(); } };
    // mousedown, not click: the textarea must keep its caret for the insertion.
    row.onmousedown = (e) => { e.preventDefault(); pickMention(it, it.type === 'directory'); };
    const ic = document.createElement('span');
    ic.className = 'mention-ic';
    ic.innerHTML = it.type === 'directory' ? ICONS.FOLDER : it.type === 'browser' ? ICONS.GLOBE : ICONS.FILEICON;
    row.appendChild(ic);
    if (it.type === 'directory') {
      row.appendChild(mentionTail('mention-dirpath', it.path));
    } else {
      const name = document.createElement('span');
      name.className = 'mention-name';
      name.textContent = it.name;
      row.appendChild(name);
      const where = it.type === 'browser' ? 'browser tab'
        : (it.path.length > it.name.length ? it.path.substring(0, it.path.length - it.name.length) : '');
      if (where) row.appendChild(mentionTail('mention-dir', where));
    }
    list.appendChild(row);
  });
  mentionEl.innerHTML = '';
  mentionEl.appendChild(list);
}

/* A right-aligned path that loses its START, not its end, when it doesn't fit. */
function mentionTail(cls, text) {
  const outer = document.createElement('span');
  outer.className = cls;
  const inner = document.createElement('span');
  inner.className = 'ltr';
  inner.textContent = text;
  outer.appendChild(inner);
  return outer;
}

function paintMentionSel() {
  const rows = mentionEl.querySelectorAll('.mention-item');
  rows.forEach((r, i) => r.classList.toggle('sel', i === mentionState.sel));
  const cur = rows[mentionState.sel];
  if (cur) cur.scrollIntoView({ block: 'nearest' });
}

/** Keys while the list is open. @returns {boolean} true when the key was the list's. */
function handleMentionKey(e) {
  const n = mentionState.items.length;
  switch (e.key) {
    case 'ArrowDown':
      if (n > 1) { e.preventDefault(); mentionState.sel = mentionState.sel < n - 1 ? mentionState.sel + 1 : 0; paintMentionSel(); }
      return true;
    case 'ArrowUp':
      if (n > 1) { e.preventDefault(); mentionState.sel = mentionState.sel > 0 ? mentionState.sel - 1 : n - 1; paintMentionSel(); }
      return true;
    case 'Tab':
    case 'Enter':
      if (e.shiftKey) return false;
      e.preventDefault();
      if (mentionState.items[mentionState.sel]) pickMention(mentionState.items[mentionState.sel], e.key === 'Tab');
      return true;
    case 'Escape':
      e.preventDefault();
      closeMention();
      return true;
  }
  return false;
}

/* The mention text for a row, or null when the path can't be written as one. A path
   that isn't a plain word is quoted: @"my file.txt". */
function mentionUnsafe(p) {
  return p.includes('"') || /[\s。、？！]@/.test(p) || /@terminal:|@browser(?=[:\s]|$)/.test(p);
}
function mentionBare(p) {
  if (/[\s:]/.test(p) || p.startsWith('agent-') || p === 'browser') return false;
  const tail = p.replace(/\/+$/, '');
  if (!/[A-Za-z0-9_]$/.test(tail)) return false;
  return tail === p || !/^[\\/]/.test(p);
}
function formatMention(path, type) {
  if (type === 'browser') return mentionUnsafe(path) ? null : '@' + path;
  if (path === '' || mentionUnsafe(path) || path.includes('#') || /^\s|\s$/.test(path)
      || path === '~' || path.startsWith('~/') || path.endsWith(' (agent)')) return null;
  return mentionBare(path) ? '@' + path : '@"' + path + '"';
}

/* Replaces the word being typed with the picked row. A folder picked with Tab or a
   click stays open for the next level down; anything else ends the mention with a
   space and closes the list. */
function pickMention(item, stepInto) {
  const at = mentionAtCaret();
  if (!at) { closeMention(); return; }
  const mention = formatMention(item.path, item.type);
  if (mention === null) { closeMention(); return; }
  const keepOpen = stepInto && item.type === 'directory';
  const v = input.value;
  const before = v.substring(0, at.start), after = v.substring(at.end);
  const insert = keepOpen ? mention : mention + (after.startsWith(' ') ? '' : ' ');
  input.value = before + insert + after;
  const caret = at.start + mention.length + (keepOpen ? 0 : 1);
  input.focus();
  input.setSelectionRange(caret, caret);
  if (!keepOpen) closeMention();
  // Grows the textarea, updates the send button, and — for a folder — lists inside it.
  input.dispatchEvent(new Event('input', { bubbles: true }));
}

/* Writes text at the caret as its own word, then opens the list for it — the + menu's
   "Add context" ("@") and "Browse the web" ("@browser:"). */
function insertMentionTrigger(text) {
  closeMenus();
  input.focus();
  const v = input.value;
  const s = input.selectionStart, e = input.selectionEnd;
  const lead = s > 0 && !/\s/.test(v[s - 1]) ? ' ' : '';
  input.value = v.substring(0, s) + lead + text + v.substring(e);
  const caret = s + lead.length + text.length;
  input.setSelectionRange(caret, caret);
  input.dispatchEvent(new Event('input', { bubbles: true }));
}

/* The + menu. Browse the web is offered only with a claude.ai sign-in. */
function openPlusMenu(anchor) {
  const web = document.getElementById('plus-browse');
  if (web) web.hidden = !(window._browserSupported && window._browserSupported());
  toggleMenu('plus-menu', anchor);
}

input.addEventListener('click', updateMentionMenu);
input.addEventListener('keyup', (e) => {
  if (e.key === 'ArrowLeft' || e.key === 'ArrowRight' || e.key === 'Home' || e.key === 'End') updateMentionMenu();
});
document.addEventListener('mousedown', (e) => {
  if (mentionState.open && !e.target.closest('#mention-menu,#input')) closeMention();
});

/* ---- mentions in sent messages ----
   An @-mention of a path, a terminal or the browser is drawn as a chip; a path chip
   opens its file in an editor. */
const MENTION_RE = /(?:^|[\s。、？！])(@"[^"]+"|@\S*[^\s.,;:!?)\]。、？！])/g;

function looksLikeMentionPath(p) {
  if (/^(types|babel|eslint|typescript-eslint|angular|vue|nestjs|testing-library|tanstack|radix-ui|reduxjs|emotion|mui|swc)\//.test(p)) return false;
  return p.includes('/') || p.includes('.') || p.includes('~');
}

/** Appends `text` to `el`, with its @-mentions as chips. */
function appendMentionText(el, text) {
  MENTION_RE.lastIndex = 0;
  let last = 0, m;
  while ((m = MENTION_RE.exec(text))) {
    const token = m[1];
    const start = m.index + (m[0].length - token.length);
    let target = token.slice(1);
    if (target.length >= 2 && target.startsWith('"') && target.endsWith('"')) target = target.slice(1, -1);
    const special = target.startsWith('terminal:') || target.startsWith('browser:');
    if ((!special && !looksLikeMentionPath(target)) || target.endsWith(' (agent)')) continue;
    if (start > last) el.appendChild(document.createTextNode(text.slice(last, start)));
    const chip = document.createElement('span');
    chip.className = 'mention-chip';
    chip.textContent = token;
    if (special) {
      chip.title = target;
    } else {
      const file = target.replace(/#L?\d+(?:-\d+)?$/, '');
      chip.title = 'Open ' + target;
      chip.setAttribute('role', 'button');
      chip.tabIndex = 0;
      const open = () => { if (window._openFileInEditor) window._openFileInEditor(file, rootPathOf(activeTab())); };
      chip.onclick = open;
      chip.onkeydown = (e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); open(); } };
    }
    el.appendChild(chip);
    last = start + token.length;
  }
  if (last < text.length) el.appendChild(document.createTextNode(text.slice(last)));
}
