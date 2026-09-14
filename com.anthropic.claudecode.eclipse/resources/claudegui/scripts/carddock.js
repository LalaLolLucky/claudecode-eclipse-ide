/* carddock.js — Bottom card dock: floats decision/question cards over the transcript at the
   composer position. */

/* ---- bottom card dock: cards float over the transcript at the composer's
   position while active (joebiden3). #messages keeps full height; we reserve
   bottom padding equal to the card height so the last message scrolls clear. ---- */
const bottomCardEl = document.getElementById('bottom-card');
function padForBottomCard() {
  messagesEl.style.paddingBottom = (bottomCardEl.offsetHeight + 8) + 'px';
}
/* Tracks BOTH growth and shrink (question-tab switches, the "Other" input
   toggling): recompute the reserved padding AND re-pin the transcript to the
   bottom, so the last message always sits exactly one gap above the card —
   never covered by a taller tab, never floating high above a shorter one. */
new ResizeObserver(() => {
  if (bottomCardEl.style.display === 'block') { padForBottomCard(); scrollBottom(); }
}).observe(bottomCardEl);
/* A pending card belongs to the conversation that raised it (its stream tab), and
   is stored ON that Tab (Tab.pendingCard) rather than in one shared global — each
   tab runs its own CLI process and can have its own card pending independently
   (e.g. tab A is mid-AskUserQuestion when tab B's background process raises its
   own approval prompt). A single shared slot would have the later card silently
   evict the earlier one: the evicted tab's card vanishes from the DOM with no
   cleanup (its keydown listener and timeout registration both leak, still armed
   against a card no longer shown), and its Java-side future is orphaned — nothing
   can ever answer it, so that tab hangs on its tool's "pending" state until the
   timeout preference (or the user interrupting the session) ends it. Only shows
   while its owning tab is active; switching away hides it (composer returns),
   switching back re-shows THAT tab's own card, not whichever was raised last. */
function showBottomCard(card, owner) {
  owner.pendingCard = card;
  renderBottomCard();
}
function renderBottomCard() {
  const composer = document.getElementById('composer');
  const t = activeTab();
  const card = t && t.pendingCard;
  if (card) {
    if (bottomCardEl.firstChild !== card) { bottomCardEl.innerHTML = ''; bottomCardEl.appendChild(card); }
    bottomCardEl.style.display = 'block';
    composer.style.display = 'none';
    // Card only shows for the active tab → autoScroll, not scrollBottom, whose background
    // guard reads an rtab that may be a stale background stream. autoScroll has no such
    // guard and obeys Scroll Lock, which a card must too: the card floats at the composer
    // position and is fully visible wherever the transcript sits, so holding still costs
    // the user nothing and moving them costs them their place.
    // padForBottomCard stays OUTSIDE the scroll decision — the reserved space is layout,
    // not movement, and it has to be right for when they do scroll back down.
    requestAnimationFrame(() => { padForBottomCard(); autoScroll(); });
  } else {
    bottomCardEl.innerHTML = '';
    bottomCardEl.style.display = 'none';
    messagesEl.style.paddingBottom = '';
    composer.style.display = '';
  }
}
/** @param {Tab} owner the tab whose OWN card is being cleared — never inferred
 *  from rtab/activeTab, so dismissing tab A's card can never touch tab B's. */
function clearBottomCard(owner) {
  if (owner) owner.pendingCard = null;
  renderBottomCard();      // hides the card, restores the composer (if this was the active tab's)
  input.focus();
  // A blocking card (AskUserQuestion / approval) suspends the turn WITHOUT ending
  // it — dismissing the card resumes the same turn, so no onStreamStart fires to
  // restore the gerund. Guarantee it here (the single dismissal choke point) so
  // the working indicator is present whenever a turn is still processing.
  if (owner) loadRender(owner);
  ensureWorking();
}

/* ---- cancel key (Eclipse binding) ----
   Java binds a real Eclipse command to Esc (default scheme) or Ctrl+G (Emacs, where
   Esc is a multi-stroke prefix Eclipse swallows before the page ever sees it) and calls
   cancelActiveCard when it fires. Each card registers its own cancel() here while it is
   up. This is ADDITIVE to the cards' in-page Escape listeners: if the Eclipse dispatcher
   route turns out not to reach us from inside WebView2, default-scheme Esc keeps working
   exactly as it did, so the worst case is no change rather than a regression.

   setCancelHint carries the label of whichever key is actually bound, so the card can
   advertise the truth instead of hardcoding "Esc". Empty means nothing is bound — the
   card then shows no hint at all rather than naming a key that does nothing. */
let cancelHint = 'Esc';
/* A STACK, not a single slot: opening the find bar over an already-open history panel
   (or any overlay-over-overlay combination — nine call sites register here) used to
   clobber whichever registration was already live, with no way back to it. Dismissing
   the newer one then left the dismiss key dead for the older one still visibly open,
   since its registration had been overwritten rather than preserved underneath. Each
   entry is {fn, isBottom, owner, notifiesJava} — see registerCardCancel for what owner
   means, and registerOverlayCancel for notifiesJava (whether popping this entry down to
   an empty stack should call _overlayOpen(false); Java-raised cards never should, since
   Java already toggled its own context around them and was never told they opened via
   _overlayOpen in the first place).
   activeCardCancel/activeCancelIsBottom/activeCancelOwner are kept as mirrors of the TOP
   entry (see syncTop) so every existing external read (find.js, ui.js) keeps working
   unchanged: an empty stack mirrors to null/false/null, exactly like the old "nothing
   registered" state. */
const overlayStack = [];
let activeCardCancel = null, activeCancelIsBottom = false, activeCancelOwner = null;
// How many entries currently on the stack were registered via registerOverlayCancel
// (notifiesJava: true) rather than registerCardCancel (false) — see registerOverlayCancel
// for why _overlayOpen must fire on THIS count's 0↔1 transitions, not the stack's own
// emptiness: a Java-raised card can push/pop while an overlay entry sits underneath it,
// and none of that may touch _overlayOpen at all. Recomputed by a full reduce() inside
// pushOverlay/popOverlay (the only two mutators) after every push/pop — the stack is
// only ever 1-3 entries deep, so this is not worth doing incrementally — so every
// register/unregister call site, including cancelActiveCard's own pop, gets the correct
// notify for free.
let notifyingCount = 0;
function syncTop() {
  const top = overlayStack.length ? overlayStack[overlayStack.length - 1] : null;
  activeCardCancel = top ? top.fn : null;
  activeCancelIsBottom = top ? top.isBottom : false;
  activeCancelOwner = top ? top.owner : null;
}
/* Pushes fn, or moves it to the top if already present (re-registering the SAME overlay
   that's already on the stack must not create a second entry for it; if it re-registers
   with a DIFFERENT notifiesJava than before, the recount below still ends up correct
   either way). */
function pushOverlay(fn, isBottom, owner, notifiesJava) {
  const i = overlayStack.findIndex(e => e.fn === fn);
  if (i !== -1) overlayStack.splice(i, 1);
  overlayStack.push({ fn, isBottom: !!isBottom, owner: owner || null, notifiesJava: !!notifiesJava });
  syncTop();
  const before = notifyingCount;
  notifyingCount = overlayStack.reduce((n, e) => n + (e.notifiesJava ? 1 : 0), 0);
  if (before === 0 && notifyingCount > 0 && window._overlayOpen) window._overlayOpen(true);
}
/* Removes fn wherever it sits in the stack, or the top entry if fn is omitted (every
   bare unregister*() call site — see their own comments for why popping the top is
   correct there: each only ever runs as a REGISTERED cancel actually firing, so it can
   only be at the top by the time it runs, or as its owning overlay's own close path with
   nothing else having registered since). Explicit-fn removal exists for the one case
   that isn't top-only: ui.js's closeMenus() can close the history panel out from under a
   LATER overlay (find opened on top of history, then the user clicked outside) — the
   panel is gone but its entry would otherwise be stranded underneath find's, and the
   next dismiss-key press would pop find correctly only to land on history's now-dead
   entry next, consuming a second press on nothing. */
function popOverlay(fn) {
  if (!overlayStack.length) return null;
  const i = fn ? overlayStack.findIndex(e => e.fn === fn) : overlayStack.length - 1;
  if (i === -1) return null;   // fn wasn't on the stack — e.g. it already removed itself
  const [entry] = overlayStack.splice(i, 1);
  syncTop();
  const before = notifyingCount;
  notifyingCount = overlayStack.reduce((n, e) => n + (e.notifiesJava ? 1 : 0), 0);
  if (before > 0 && notifyingCount === 0 && window._overlayOpen) window._overlayOpen(false);
  return entry;
}

/* Every hint on screen repaints itself when the binding changes, rather than waiting to be
   rebuilt. Java pushes a new label the moment Eclipse's BindingManager fires — switching
   scheme in Preferences and hitting Apply must update visible text there and then, not on
   the next card. Keyed by element and self-pruning: a hint whose card is gone is simply
   dropped on the next pass, so no surface needs teardown wiring. */
const hintPainters = new Map();   // element -> () => void
function registerHintPainter(el, paint) {
  if (!el) return;
  hintPainters.set(el, paint);
  // Guarded like the refresh pass: a hint that fails to draw must not take the card it
  // belongs to down with it. The hint is the least important thing on screen.
  try { paint(); } catch (e) {}
}
function refreshCancelHints() {
  for (const [el, paint] of hintPainters) {
    if (!el.isConnected) { hintPainters.delete(el); continue; }
    try { paint(); } catch (e) {}
  }
}
window.setCancelHint = function(label) {
  const next = label || '';
  if (next === cancelHint) return;
  cancelHint = next;
  refreshCancelHints();
};
/* Raw key name for hints that embed it in their own sentence ("… to close", "(Esc)"). */
function cancelKeyName() { return cancelHint; }
function cancelHintText() { return cancelHint ? cancelHint + ' to cancel' : ''; }

/* Java-raised blocking cards: Java already activated the key context around its own
   future.get(), so these must NOT notify it again (see registerOverlayCancel for the
   ones that must — pushOverlay/popOverlay's own notifyingCount bookkeeping is what keeps
   these two silent while that's still true no matter what else is on the stack). owner
   is the tab this card belongs to (the same owner showBottomCard was given) — needed
   even with a stack, since a card raised on a background tab must never be the one that
   fires when the dismiss key is pressed over a DIFFERENT tab's own card
   (cancelActiveCard's owner check is what actually enforces that; owner is only carried
   here for it to read off the top entry). */
function registerCardCancel(fn, owner) { pushOverlay(fn, true, owner, false); }
function unregisterCardCancel() { popOverlay(); }

/* Page-local overlays (advisor card, rewind picker, lightbox, find bar, history panel).
   Java cannot know these are open — nothing on its side raised them — so the page has to
   say so, or the key context never activates and the key stays dead however honest the
   hint is. owner: see registerCardCancel — only meaningful when isBottomCard, since a
   non-bottom overlay isn't tab-owned. fn on unregister: see popOverlay's own comment —
   omit it to pop the top (the common case, valid whenever this call IS a registered
   cancel actually firing), pass it to remove a specific entry that may no longer be on
   top (e.g. ui.js's closeMenus() closing the history panel while a later overlay sits
   above it). */
function registerOverlayCancel(fn, isBottomCard, owner) { pushOverlay(fn, isBottomCard, owner, true); }
function unregisterOverlayCancel(fn) { popOverlay(fn); }

// One dismiss GESTURE (one Ctrl+G / Esc press) must cancel at most one overlay. The old
// single-slot design got this for free by accident: a double-fire of the Eclipse command
// (see DismissCardHandler's own comment — tolerated as a known, harmless quirk) found
// activeCardCancel already null on its second call and no-opped. The stack has no such
// accidental protection — a second call within the same gesture finds a NEW top entry
// (whatever was underneath the first) and cancels that too, observed as one Ctrl+G
// closing both the find bar AND the history panel underneath it. lastCancelAt makes the
// one-gesture-one-cancel invariant explicit instead of relying on a side effect that no
// longer holds. 50ms comfortably covers the ~24ms gap measured between the two calls in
// /tmp/find-debug.log while staying far below a human's fastest deliberate double-press.
let lastCancelAt = 0;
window.cancelActiveCard = function() {
  if (!activeCardCancel) return;
  if (Date.now() - lastCancelAt < 50) return;
  // Bottom cards only: one parked on a background tab must not vanish because a key was
  // pressed over another conversation. Mirrors renderBottomCard's own visibility test,
  // now split into two checks since each tab tracks its own pendingCard instead of one
  // shared pendingCard/pendingCardOwner pair: the ACTIVE tab must actually have a card
  // showing, AND it must be the same tab that registered this cancel — two different
  // background tabs can each have their own card pending, so "some card is showing" is
  // not enough on its own; it has to be THIS card's owner specifically, or switching to
  // tab A while the top entry is still tab B's would fire B's cancel from A's screen.
  // Overlays (rewind, lightbox) are not tab-owned, so the test does not apply to them.
  // Reads the TOP entry's own isBottom/owner (the mirrors), so this guard is always
  // evaluated against whichever overlay would actually be cancelled below, not some
  // other entry buried in the stack.
  const t = activeTab();
  if (activeCancelIsBottom && (!t || !t.pendingCard || activeCancelOwner !== t)) return;
  const entry = popOverlay();   // notifyingCount bookkeeping (and _overlayOpen) handled inside
  if (!entry) return;
  // Written here, not at function entry: an early return above (nothing registered, the
  // bottom-card visibility guard, popOverlay finding nothing) must not arm the guard for
  // a call that didn't actually cancel anything.
  lastCancelAt = Date.now();
  entry.fn();
};

/* ---- server-side card teardown ----
   Two ways a card stops being answerable without the user touching it, and both
   arrive from Java because only Java can know about them:

     'timeout'   — the per-card timeout preference expired. Java has ALREADY
                   answered the CLI itself (deny / dismissed) before this runs.
     'cancelled' — the CLI withdrew the request. Under Remote Control that means
                   the decision was made on the phone or on claude.ai, where the
                   same prompt is shown; it also covers a turn that ended without
                   this prompt (interrupt, or a hard failure). Nothing was
                   answered here and nothing will be.

   Each card registers one same-shape cleanup here (indexed by reqId) right
   before showBottomCard and unregisters it the moment it resolves itself (click
   / Enter / Esc), so a click racing either teardown can't double-fire. Neither
   reason may call _decide/_answerQuestion: in the timeout case that reqId is no
   longer pending on the Java side, and in the cancelled case the CLI has stopped
   listening for it. Both are presentation-only. */
const pendingCardTimeouts = new Map();  // reqId -> (reason) => void
function registerCardTimeout(reqId, onTornDown) { pendingCardTimeouts.set(reqId, onTornDown); }
function unregisterCardTimeout(reqId) { pendingCardTimeouts.delete(reqId); }
function tearDownCard(reqId, reason) {
  const fn = pendingCardTimeouts.get(reqId);
  if (!fn) return;   // already resolved by the user, or not this page's card
  pendingCardTimeouts.delete(reqId);
  fn(reason);
}
window.dismissTimedOutCard = function(reqId) { tearDownCard(reqId, 'timeout'); };
/* The CLI withdrew this request — see 'cancelled' above. */
window.cancelPendingCard = function(reqId) { tearDownCard(reqId, 'cancelled'); };

