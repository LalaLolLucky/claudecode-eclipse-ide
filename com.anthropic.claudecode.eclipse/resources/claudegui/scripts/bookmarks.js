/* bookmarks.js — Bookmarked replies (joebiden2): the two buttons under each of Claude's
   replies (copy, bookmark), and the Bookmarks panel beside the conversation.

   A bookmark belongs to one conversation and points at one reply: the transcript line
   the reply is (its uuid). The core keeps them (bookmarks.rs), one file per
   conversation; the text shown in the panel is the reply's own, never a copy.

   Only Claude's replies are marked — the paragraphs appendAssistant streams and
   appendTextStatic rebuilds. Tool lines, thinking, a subagent's log and the plugin's
   own notices are built elsewhere and never reach sealReply.

   The panel's open/closed state and its selected row are the TAB's (t.bmVisible,
   t.bmSelected), as the VS Code extension keeps them per session; the panel shows the
   tab in front. Its size is the user's, shared by every tab. */

/* Whether the view behind the page keeps bookmarks at all: a native library from before
   them answers null, and the page then offers none (the copy button needs nothing). */
let bookmarksOn = null;
function bookmarksAvailable() {
  if (bookmarksOn === null) {
    try { bookmarksOn = !!window._bookmarks && typeof window._bookmarks('') === 'string'; }
    catch (e) { bookmarksOn = false; }
  }
  return bookmarksOn;
}

/* ---- the buttons under a reply ---- */

/* Marks a finished reply as Claude's own and gives it its buttons: copy, then bookmark.
   `text` is the reply as Claude wrote it (markdown), which is what Copy copies and what
   identifies the reply in the transcript. */
function sealReply(el, text) {
  if (!el || el.classList.contains('reply') || !text || !text.trim()) return;
  el.classList.add('reply');
  el._replyText = text;
  const row = document.createElement('div'); row.className = 'a-actions';
  row.appendChild(makeCopyBtn(() => el._replyText));
  const mark = document.createElement('button');
  mark.type = 'button'; mark.className = 'bm-btn';
  mark.onclick = (e) => { e.stopPropagation(); toggleReplyBookmark(el); };
  row.appendChild(mark);
  el.appendChild(row);
  el.addEventListener('mouseenter', () => lateReplyId(el));
  paintReplyMark(el, null);
}
/* A reply still without its transcript line when the pointer reaches it — the CLI had
   not written the line when its turn ended — asks once more, so its bookmark button is
   there to be clicked. Once per reply: one the transcript never holds must not cost a
   read of it on every pass of the pointer. */
function lateReplyId(el) {
  if (el.dataset.rid || el._replyAsked) return;
  const t = tabs.find(x => x.pane && x.pane.contains(el));
  if (!t || t.streaming) return;
  el._replyAsked = true;
  backfillReplyIds(t);
  t.pane.querySelectorAll('.a-item.reply').forEach(x => paintReplyMark(x, t));
}
/* The bookmark button as it stands: there at all only once the reply's transcript line
   is known, filled and staying in view while the reply is bookmarked. */
function paintReplyMark(el, t) {
  const mark = el.querySelector(':scope > .a-actions > .bm-btn');
  if (!mark) return;
  const rid = el.dataset.rid;
  const on = !!(t && rid && tabBookmarks(t).some(b => b.uuid === rid));
  mark.hidden = !(rid && bookmarksAvailable());
  mark.classList.toggle('on', on);
  mark.title = on ? 'Remove bookmark' : 'Bookmark response';
  mark.setAttribute('aria-label', mark.title);
  mark.innerHTML = on ? ICONS.BOOKMARKED : ICONS.BOOKMARK;
}
/* A reply streamed this run has no transcript line yet — the CLI writes it while the turn
   runs — so, as backfillMessageIds does for the bubbles, each reply still without one is
   matched to the transcript's replies by its text, each line claimed at most once. */
function backfillReplyIds(t) {
  if (!t || !t.pane || !t.sessionId || !window._replyIds || !bookmarksAvailable()) return;
  const blank = [].slice.call(t.pane.querySelectorAll('.a-item.reply:not([data-rid])'));
  if (!blank.length) return;
  let list = [];
  // With the tab's folder, for the reason backfillMessageIds gives.
  try { list = JSON.parse(window._replyIds(t.sessionId, rootPathOf(t)) || '[]') || []; } catch (e) { return; }
  const taken = new Set();
  t.pane.querySelectorAll('.a-item.reply[data-rid]').forEach(el => taken.add(el.dataset.rid));
  const free = list.filter(r => r && r.id && !taken.has(r.id));
  blank.forEach(el => {
    // Exactly as written first; a reply differing only by the space around it is the same one.
    let i = free.findIndex(r => r.text === el._replyText);
    if (i < 0) i = free.findIndex(r => String(r.text).trim() === el._replyText.trim());
    if (i < 0) return;
    el.dataset.rid = free[i].id;
    const at = Date.parse(free[i].at || '');
    if (!isNaN(at)) el._replyAt = at;
    free.splice(i, 1);
  });
}
/* Brings a tab's replies up to date: buttons on the ones that have finished, a
   transcript line for each, and the marks of the ones that are bookmarked. Called when
   a turn ends (however it ended) and when a conversation has been rebuilt from history. */
function refreshReplies(t) {
  if (!t || !t.pane) return;
  t.pane.querySelectorAll('.a-item').forEach(el => {
    if (el._replyText && !el.classList.contains('reply')) sealReply(el, el._replyText);
  });
  backfillReplyIds(t);
  t.pane.querySelectorAll('.a-item.reply').forEach(el => paintReplyMark(el, t));
  if (t === activeTab()) renderBookmarks();
}
/* The reply a bookmark points at, when it is on screen in this tab. */
function replyEl(t, uuid) {
  if (!t || !t.pane || !uuid) return null;
  const all = t.pane.querySelectorAll('.a-item.reply[data-rid]');
  for (let i = 0; i < all.length; i++) if (all[i].dataset.rid === uuid) return all[i];
  return null;
}

/* ---- a conversation's bookmarks ---- */

/* The tab's bookmarks, read once per conversation (a tab's session id changes when it
   takes on another conversation). */
function tabBookmarks(t) {
  if (!t) return [];
  if (t.bmFor !== (t.sessionId || '')) {
    t.bmFor = t.sessionId || '';
    t.bookmarks = []; t.bmTexts = {}; t.bmSelected = null;
    if (t.sessionId && bookmarksAvailable()) {
      try { t.bookmarks = JSON.parse(window._bookmarks(t.sessionId) || '[]') || []; } catch (e) { t.bookmarks = []; }
    }
  }
  return t.bookmarks || [];
}
function toggleReplyBookmark(el) {
  const t = tabs.find(x => x.pane && x.pane.contains(el));
  const uuid = el.dataset.rid;
  if (!t || !uuid) return;
  setBookmark(t, uuid, !tabBookmarks(t).some(b => b.uuid === uuid), el._replyAt || 0);
}
/* Bookmarks a reply or takes its bookmark away, then shows what the core now holds —
   so a bookmark that could not be written is not shown as kept. */
function setBookmark(t, uuid, on, writtenAt) {
  if (!t || !t.sessionId || !window._setBookmark) return;
  let out = null;
  try { out = JSON.parse(window._setBookmark(t.sessionId, uuid, on, writtenAt || 0) || 'null'); } catch (e) {}
  if (!out || !Array.isArray(out.bookmarks)) return;
  tabBookmarks(t);
  t.bookmarks = out.bookmarks;
  if (!on && t.bmSelected === uuid) t.bmSelected = null;
  if (on && !out.ok) addSystemTo(t, '⚠ This bookmark couldn\'t be written to disk.');
  t.pane.querySelectorAll('.a-item.reply').forEach(el => paintReplyMark(el, t));
  if (t === activeTab()) renderBookmarks();
}
/* A bookmarked reply's text: the reply's own when it is on screen, otherwise read from
   the transcript (once), and null when the transcript does not hold it either. */
function bookmarkText(t, uuid) {
  const el = replyEl(t, uuid);
  if (el) return el._replyText;
  if (!t.bmTexts) t.bmTexts = {};
  if (!(uuid in t.bmTexts)) {
    const missing = tabBookmarks(t).map(b => b.uuid).filter(id => !(id in t.bmTexts) && !replyEl(t, id));
    let read = {};
    try { read = JSON.parse(window._bookmarkTexts(t.sessionId, rootPathOf(t), JSON.stringify(missing)) || '{}') || {}; } catch (e) {}
    missing.forEach(id => { t.bmTexts[id] = typeof read[id] === 'string' ? read[id] : null; });
  }
  return t.bmTexts[uuid];
}

/* ---- what a row says ---- */

/* A reply in one line, for its row: its first line that says something, without the
   markdown that would only be noise there. Code is passed over unless code is all
   there is. */
function bookmarkPreview(text) {
  let inCode = false, firstCode;
  const lines = String(text).split('\n');
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i].trim();
    if (line.indexOf('```') === 0) { inCode = !inCode; continue; }
    if (line === '') continue;
    if (inCode) { if (firstCode === undefined) firstCode = line; continue; }
    // Cut to length, and never through the middle of a character two code units long.
    let head = line.slice(0, 300);
    const last = head.charCodeAt(head.length - 1);
    if (line.length > 300 && last >= 0xD800 && last <= 0xDBFF) head = head.slice(0, -1);
    const plain = head
      .replace(/^#+\s+|^[-*>]\s+|^\d+\.\s+/, '')
      .replace(/(\*\*|\*)(\S(?:.*?\S)?)\1/g, '$2')
      .replace(/`([^`]*)`/g, '$1')
      .trim();
    if (plain !== '' && !/^[-*_=\s]+$/.test(plain)) return plain;
  }
  if (firstCode !== undefined) return firstCode;
  // Nothing but rules and blank lines: its first line, such as it is.
  const whole = String(text).trim(), end = whole.indexOf('\n');
  return (end < 0 ? whole : whole.slice(0, end)).trim();
}
/* How long ago a reply was written, as its row shows it. */
function bookmarkAge(at, now) {
  const secs = Math.floor(((now === undefined ? Date.now() : now) - at) / 1000);
  if (secs < 60) return 'just now';
  const mins = Math.floor(secs / 60);
  if (mins < 60) return mins + 'm ago';
  const hours = Math.floor(mins / 60);
  if (hours < 24) return hours + 'h ago';
  return Math.floor(hours / 24) + 'd ago';
}

/* ---- the panel ---- */

function toggleBookmarksPanel() {
  const t = activeTab();
  if (!t || !bookmarksAvailable()) return;
  t.bmVisible = !t.bmVisible;
  // Opened: read again, another Eclipse may have bookmarked in the same conversation. The
  // row that was picked stays picked if it is still there.
  if (t.bmVisible) {
    const picked = t.bmSelected;
    t.bmFor = undefined;
    refreshReplies(t);
    if (tabBookmarks(t).some(b => b.uuid === picked)) t.bmSelected = picked;
  }
  renderBookmarks();
}
function hideBookmarksPanel() {
  const t = activeTab();
  if (t) t.bmVisible = false;
  renderBookmarks();
}
/* Paints the toolbar button and the panel for the tab in front. */
function renderBookmarks() {
  const panel = document.getElementById('bookmarks-panel');
  const btn = document.getElementById('bookmarks-toggle');
  if (!panel) return;
  const t = activeTab();
  const available = bookmarksAvailable();
  const shown = available && !!(t && t.bmVisible);
  if (btn) {
    btn.style.display = available ? '' : 'none';
    btn.classList.toggle('active', shown);
    btn.setAttribute('aria-expanded', shown ? 'true' : 'false');
    btn.innerHTML = shown ? ICONS.BOOKMARKED : ICONS.BOOKMARK;
  }
  panel.hidden = !shown;
  document.body.classList.toggle('bm-open', shown);
  applyBookmarksPanelSize();
  if (shown) renderBookmarksBody(t);
  fitBottomCardToBookmarks();
}
function bmMessage(text, extraClass) {
  const p = document.createElement('p'); p.className = 'bm-msg' + (extraClass ? ' ' + extraClass : '');
  p.textContent = text;
  return p;
}
/* Whether a bookmark is of a reply the view is keeping out of sight: one from before the
   last compaction, while "Hide messages from before a compaction" is on (stream.js).
   Left out of the panel, not removed: it is listed again once the preference is off. */
function bookmarkHidden(t, uuid) {
  if (!hidingBeforeCompaction()) return false;
  // Of the part of the conversation that was not even read (history.js): told by its id.
  if (earlierHolds(t, uuid)) return true;
  const el = replyEl(t, uuid);
  return !!(el && el.closest('.pre-compact'));
}
function renderBookmarksBody(t) {
  const body = document.getElementById('bm-body');
  body.innerHTML = '';
  // A conversation still being opened has not said yet which replies are on screen and
  // which are hidden; the list is painted when it has (history.js, openingDone).
  if (t.opening) { body.appendChild(bmMessage('Loading…')); return; }
  const marks = tabBookmarks(t).filter(b => !bookmarkHidden(t, b.uuid));
  if (!marks.length) {
    body.appendChild(bmMessage('Bookmark a response with the bookmark button under it to keep it here.'));
    return;
  }
  const list = document.createElement('ul'); list.className = 'bm-list';
  marks.forEach(b => {
    const text = bookmarkText(t, b.uuid);
    const li = document.createElement('li');
    const row = document.createElement('button');
    row.type = 'button'; row.className = 'bm-row' + (b.uuid === t.bmSelected ? ' selected' : '');
    row.dataset.uuid = b.uuid;
    if (b.uuid === t.bmSelected) row.setAttribute('aria-current', 'true');
    const label = document.createElement('span'); label.className = 'bm-row-text';
    label.textContent = text === null ? 'Response can\'t be shown' : bookmarkPreview(text);
    row.appendChild(label);
    if (typeof b.writtenAt === 'number') {
      const time = document.createElement('span'); time.className = 'bm-row-time';
      time.dataset.at = String(b.writtenAt);
      time.textContent = bookmarkAge(b.writtenAt);
      row.appendChild(time);
    }
    row.onclick = () => { t.bmSelected = b.uuid; renderBookmarks(); };
    li.appendChild(row); list.appendChild(li);
  });
  body.appendChild(list);

  const picked = marks.find(b => b.uuid === t.bmSelected);
  if (!picked) { body.appendChild(bmMessage('Click a bookmark to show it here.', 'bm-pick')); return; }
  const text = bookmarkText(t, picked.uuid);
  const reprint = document.createElement('div'); reprint.className = 'bm-reprint';
  const actions = document.createElement('div'); actions.className = 'bm-reprint-actions';
  // In the conversation: on screen, or in the part of it that is fetched on being asked for.
  if (replyEl(t, picked.uuid) || earlierHolds(t, picked.uuid)) {
    const jump = document.createElement('button');
    jump.type = 'button'; jump.className = 'bm-link'; jump.textContent = 'Show in conversation';
    jump.onclick = () => showBookmarkedReply(t, picked.uuid);
    actions.appendChild(jump);
  } else {
    const gone = document.createElement('span'); gone.className = 'bm-notshown';
    gone.textContent = 'No longer shown in the conversation';
    actions.appendChild(gone);
  }
  const remove = document.createElement('button');
  remove.type = 'button'; remove.className = 'bm-remove'; remove.title = 'Remove bookmark';
  remove.setAttribute('aria-label', 'Remove bookmark');
  remove.innerHTML = ICONS.BOOKMARKED;
  remove.onclick = () => setBookmark(t, picked.uuid, false, 0);
  actions.appendChild(remove);
  reprint.appendChild(actions);
  if (text === null) {
    reprint.appendChild(bmMessage('This response can\'t be shown because its conversation history can\'t be found or read.'));
  } else {
    const said = document.createElement('div'); said.className = 'bm-reprint-text a-body';
    said.innerHTML = renderMarkdown(text);
    reprint.appendChild(said);
  }
  body.appendChild(reprint);
}
/* Brings a bookmarked reply into view in its tab's conversation. One in the part before
   the last compaction that was not read when the conversation was reopened is fetched
   first, with the rest of that part, and shown when it is in. */
function showBookmarkedReply(t, uuid) {
  const el = replyEl(t, uuid);
  if (el) { showReplyInConversation(el); return; }
  if (!earlierHolds(t, uuid)) return;
  setPreCompactOpen(t.pane, true);   // asks for the part
  fetchEarlierPart(t, () => { const now = replyEl(t, uuid); if (now) showReplyInConversation(now); });
}
/* Brings a reply into view in the conversation. Through find.js's scrollToMatch, which
   moves the one container that has to move whatever the reply holds. */
function showReplyInConversation(el) {
  // A reply from before the last compaction is under "Messages before compaction",
  // which has to be open for it to be anywhere.
  if (el.closest('.pre-compact')) setPreCompactOpen(el.closest('.pane'), true);
  const range = document.createRange();
  range.selectNodeContents(el);
  if (typeof scrollToMatch === 'function') scrollToMatch(range);
  else el.scrollIntoView({ block: 'center' });
}
/* The rows' ages are relative, so they are kept current while the panel is open. */
setInterval(() => {
  document.querySelectorAll('#bm-body .bm-row-time').forEach(el => {
    el.textContent = bookmarkAge(Number(el.dataset.at));
  });
}, 60000);

/* ---- the panel's size ----
   Beside the conversation the panel has a width, and under it — a view narrower than
   500px — a height. Each is the user's own once dragged, kept across restarts. */
const BM_MIN_WIDTH = 200, BM_MIN_HEIGHT = 140, BM_MAX_SHARE = 0.7, BM_KEY_STEP = 16;
const bmStackedQuery = window.matchMedia ? window.matchMedia('(max-width: 500px)') : null;
let bmSize = { width: null, height: null };
try {
  const kept = JSON.parse(localStorage.getItem('claude.bookmarksPanelSize') || 'null');
  if (kept && typeof kept === 'object') {
    if (typeof kept.width === 'number') bmSize.width = kept.width;
    if (typeof kept.height === 'number') bmSize.height = kept.height;
  }
} catch (e) {}
function bmStacked() { return !!(bmStackedQuery && bmStackedQuery.matches); }
function applyBookmarksPanelSize() {
  const panel = document.getElementById('bookmarks-panel');
  if (!panel) return;
  const stacked = bmStacked();
  const size = stacked ? bmSize.height : bmSize.width;
  // Unset, the stylesheet's own default applies.
  panel.style.flexBasis = size === null ? ''
    : 'clamp(' + (stacked ? BM_MIN_HEIGHT : BM_MIN_WIDTH) + 'px, ' + size + 'px, ' + (BM_MAX_SHARE * 100) + '%)';
}
function setBookmarksPanelSize(size, keep) {
  const panel = document.getElementById('bookmarks-panel');
  const within = panel && panel.parentElement;
  if (!within) return;
  const stacked = bmStacked();
  const most = Math.floor((stacked ? within.offsetHeight : within.offsetWidth) * BM_MAX_SHARE);
  const least = stacked ? BM_MIN_HEIGHT : BM_MIN_WIDTH;
  const fitted = Math.round(Math.max(least, Math.min(size, Math.max(least, most))));
  if (stacked) bmSize.height = fitted; else bmSize.width = fitted;
  applyBookmarksPanelSize();
  fitBottomCardToBookmarks();
  if (keep) { try { localStorage.setItem('claude.bookmarksPanelSize', JSON.stringify(bmSize)); } catch (e) {} }
}
(function wireBookmarksResize() {
  const handle = document.getElementById('bm-resize');
  const panel = document.getElementById('bookmarks-panel');
  if (!handle || !panel) return;
  let drag = null;
  handle.addEventListener('pointerdown', (e) => {
    if (e.button !== 0) return;
    const box = panel.getBoundingClientRect();
    const stacked = bmStacked();
    // The page may be zoomed (#zoom-root is scaled): a pointer moves in screen pixels,
    // the panel is sized in its own.
    const scale = (stacked ? box.height / panel.offsetHeight : box.width / panel.offsetWidth) || 1;
    drag = { id: e.pointerId, stacked: stacked, scale: scale, far: stacked ? box.bottom : box.right };
    try { handle.setPointerCapture(e.pointerId); } catch (err) {}
    handle.classList.add('resizing');
    e.preventDefault();
  });
  handle.addEventListener('pointermove', (e) => {
    if (!drag || e.pointerId !== drag.id) return;
    setBookmarksPanelSize((drag.far - (drag.stacked ? e.clientY : e.clientX)) / drag.scale, false);
  });
  const done = (e) => {
    if (!drag || e.pointerId !== drag.id) return;
    drag = null;
    handle.classList.remove('resizing');
    const stacked = bmStacked();
    setBookmarksPanelSize(stacked ? panel.offsetHeight : panel.offsetWidth, true);
  };
  handle.addEventListener('pointerup', done);
  handle.addEventListener('pointercancel', done);
  handle.addEventListener('keydown', (e) => {
    const stacked = bmStacked();
    const grow = stacked ? 'ArrowUp' : 'ArrowLeft', shrink = stacked ? 'ArrowDown' : 'ArrowRight';
    if (e.key !== grow && e.key !== shrink) return;
    e.preventDefault();
    const now = stacked ? panel.offsetHeight : panel.offsetWidth;
    setBookmarksPanelSize(now + (e.key === grow ? BM_KEY_STEP : -BM_KEY_STEP), true);
  });
  if (bmStackedQuery && bmStackedQuery.addEventListener) {
    bmStackedQuery.addEventListener('change', () => { applyBookmarksPanelSize(); fitBottomCardToBookmarks(); });
  }
  if (window.ResizeObserver) new ResizeObserver(fitBottomCardToBookmarks).observe(panel);
})();
/* A decision card docks over the composer, across the foot of the whole view
   (#bottom-card is fixed, outside #zoom-root). With the panel open it keeps to the
   conversation's side of it. */
function fitBottomCardToBookmarks() {
  const card = document.getElementById('bottom-card');
  const panel = document.getElementById('bookmarks-panel');
  if (!card || !panel) return;
  const box = panel.hidden ? null : panel.getBoundingClientRect();
  const stacked = bmStacked();
  card.style.right = box && !stacked ? box.width + 'px' : '';
  card.style.bottom = box && stacked ? box.height + 'px' : '';
}
