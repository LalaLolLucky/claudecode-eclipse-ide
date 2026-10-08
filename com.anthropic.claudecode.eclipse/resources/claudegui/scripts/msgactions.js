/* msgactions.js — Per-message hover actions on a sent user bubble (joebiden8):
   round badges over the bubble's top-right corner, the undo badge opening
   "Fork conversation from here / Rewind code to here / Fork conversation and
   rewind code", plus the delete badge beside it.

   The undo badge keeps the exact position the reference puts it in, so the
   delete badge is added to its LEFT rather than displacing it.

   Every action targets one transcript line, so a bubble is only actionable once
   its uuid is known (data-mid). History-loaded bubbles carry it from the render
   item; a bubble sent THIS run gets it from backfillMessageIds once the CLI has
   written the line. */

/** The two badges — delete on the left, message actions on the right. */
function makeMsgActions() {
  const wrap = document.createElement('div'); wrap.className = 'msg-actions';
  const del = document.createElement('span');
  del.className = 'msg-act danger'; del.title = 'Delete message';
  del.innerHTML = ICONS.TRASH;
  del.onclick = (e) => { e.stopPropagation(); msgDelete(wrap.parentNode); };
  const acts = document.createElement('span');
  acts.className = 'msg-act'; acts.title = 'Message actions';
  acts.innerHTML = ICONS.UNDO;
  acts.onclick = (e) => { e.stopPropagation(); openMsgMenu(acts); };
  wrap.appendChild(del); wrap.appendChild(acts);
  return wrap;
}

/* The menu, in the reference's order. */
const MSG_ACTIONS = [
  ['Fork conversation from here', msgForkOnly],
  ['Rewind code to here', msgRewindCode],
  ['Fork conversation and rewind code', msgForkAndRewind],
];
function openMsgMenu(anchor) {
  const menu = document.getElementById('msg-menu');
  if (!menu) return;
  const reopen = menu.classList.contains('open') && menu._anchor === anchor;
  closeMenus();
  if (reopen) return;
  const box = anchor.closest('.user-msg');
  menu.innerHTML = '';
  MSG_ACTIONS.forEach(([label, fn]) => {
    const mi = document.createElement('div'); mi.className = 'mi'; mi.textContent = label;
    mi.onclick = (e) => { e.stopPropagation(); closeMenus(); fn(box); };
    menu.appendChild(mi);
  });
  menu._anchor = anchor;
  menu.classList.add('open');
  // Keep the badges up while their menu is open (the bubble may lose :hover).
  const wrap = anchor.parentNode;
  if (wrap && wrap.classList) wrap.classList.add('open');
  positionMenu(menu, anchor);
  openMenuEl = menu; openAnchor = anchor;
}

/** The tab + transcript id a bubble's actions apply to, or null when unknown. */
function msgTarget(box) {
  const t = activeTab();
  const mid = (box && box.dataset) ? box.dataset.mid : '';
  return (t && mid) ? { tab: t, mid: mid, box: box } : null;
}
function msgForkOnly(box) {
  const g = msgTarget(box);
  if (g) forkFrom(g.tab, g.mid, window._rewindForkOnly, 'Fork failed: ');
}
function msgRewindCode(box) {
  const g = msgTarget(box);
  if (g) openConfirmPhase(g.tab, g.mid, 'code', g.box);
}
function msgForkAndRewind(box) {
  const g = msgTarget(box);
  if (g) openConfirmPhase(g.tab, g.mid, 'confirm', g.box);
}
function msgDelete(box) {
  const g = msgTarget(box);
  if (!g) return;
  // A delete drops this tab's process (so the model stops holding the deleted
  // text), which would cut off a reply that's still streaming — make the user
  // finish or stop it first rather than silently killing their own turn.
  if (g.tab.streaming) {
    addSystem('⚠ Finish or stop the current response before deleting a message.');
    return;
  }
  openConfirmPhase(g.tab, g.mid, 'delete', g.box);
}

/* A just-sent bubble has no transcript id yet — the CLI writes its line while the
   turn runs. Position can't be used to pair them: a message QUEUED mid-stream is
   on screen before its line exists, so the bubbles and the id list differ in
   length and pairing by index (from either end) hands a bubble the wrong line —
   which would then delete the wrong message. So match on text, claim each id at
   most once, and leave a bubble alone when nothing matches (its line simply isn't
   written yet; the next turn's backfill picks it up).

   The stored text is raw, the bubble shows the stripped form, so compare through
   stripMeta — which is also what makes an <ide_selection> preamble or a
   /command wrapper line up with what the bubble renders.

   Only the lines of what the tab has drawn are up for claiming. A conversation reopened
   from its last compaction on has the messages before it in its transcript and not in
   its pane (history.js, t.earlier): left in, a new "continue" would claim the line of an
   old "continue" from up there, and Delete or Rewind on it would act on that one. The
   same goes for a conversation whose older messages are still being drawn, so that one
   waits until they are (openingDone).

   @returns {Object[]|null} the transcript's messages as the view lists them, null when
     they were not read */
function backfillMessageIds(t) {
  if (!t || !t.pane || !t.sessionId || !window._messageIds) return null;
  let list = [];
  // With the tab's folder: this runs at the end of a turn in whichever tab it ended, and
  // Java would otherwise read the conversation of the folder in front.
  try { list = JSON.parse(window._messageIds(t.sessionId, rootPathOf(t)) || '[]') || []; } catch (e) { return null; }
  if (t.opening) return list;   // read, for whoever asked; nothing is claimed yet
  const taken = new Set();
  t.pane.querySelectorAll('.user-msg[data-mid]').forEach(b => taken.add(b.dataset.mid));
  const free = list.filter(m => m && m.id && !taken.has(m.id) && !inUnreadPart(t, m));
  t.pane.querySelectorAll('.user-msg:not([data-mid])').forEach(box => {
    const body = box.querySelector('.body');
    const shown = body ? body.textContent : '';
    const i = free.findIndex(m => stripMeta(m.text || '') === shown);
    if (i < 0) return;
    box.dataset.mid = free[i].id;
    free.splice(i, 1);
  });
  return list;
}
