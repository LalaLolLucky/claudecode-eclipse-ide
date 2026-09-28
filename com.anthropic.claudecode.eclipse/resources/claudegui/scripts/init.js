/* Boot sequence — the top-level statements that used to run inline, in their
   ORIGINAL relative order. Runs last, after every declaration file has loaded.
   buildActionsSlash stays last (it needs SLASH_COMMANDS).

   The boot reads what it opens with from the view's functions (_defaultRoot,
   _savedViewState, _thinkingOnStartup, ...), so it runs only once every one of them is
   there. Edge (Windows) and macOS WebKit define them before this page's scripts run, so
   there it runs right here, as the page is read, as it always has. WebKitGTK (Linux,
   FreeBSD) defines them only after the page has been read; a boot then would get
   nothing back and lose it for good — the tabs to restore, the workspace root, the
   bookmarks — so the page waits, and the view has it boot once the load has completed
   (ClaudeGuiView.bootPage). Opened in a plain browser, with no view behind it, add
   ?nohost to the address and it boots without them. */

let ccBooted = false;

/* Whether every function the view made for the page is defined. _pageFunctions names
   them all, so a function added to the view is waited for with no list to keep here. */
function ccHostReady() {
  if (typeof window._pageFunctions !== 'function') return false;
  let names = null;
  try { names = JSON.parse(_pageFunctions()); } catch (e) { return false; }
  return Array.isArray(names) && names.every(n => typeof window[n] === 'function');
}

/* Boots the page if it can: called here, and by the view until it has booted. */
function ccBootIfReady() {
  if (!ccBooted && (ccHostReady() || /[?&]nohost\b/.test(location.search))) ccBoot();
  return ccBooted;
}

function ccBoot() {
  if (ccBooted) return;
  ccBooted = true;
  try {
    /* What two scripts did as they were read, done now if the view's functions were not
       there yet (WebKitGTK); where they were, both are no-ops. */
    readInitialContext();
    announceEditOps();

    initModelConfig();
    /* Before createTab: the workspace root has to exist for the first conversation to
       belong to, and #tabs renders only the ACTIVE root's tabs. */
    initRoots();
    /* Reopen the roots and conversations this workspace was last closed with; only when
       there is nothing to restore does the view open its usual single empty conversation. */
    const restored = restoreViewState();
    if (!restored) createTab();
    updateCtxChip();
    /* Paint the composer from the module defaults ONLY when nothing was restored.
       restoreViewState() ends in switchTab() -> applyTabSettings(), which has already
       painted the restored conversation's effort and thinking. Re-running these two
       afterwards is not a harmless repaint: setEffort() writes t.effortIdx and calls
       persistTabPrefs(), and unlike applyTabSettings' call it passes no {force:true},
       so a restored xhigh/max pair gets re-clamped through maxEffortIdx() and the
       clamped value is written straight back into the sidecar. */
    if (!restored) { setEffort(effortIdx); updateThinkingCheck(); }

    /* Draw the command menu and ask what the folder offers (cmdmenu.js). */
    cmdMenuInit();
    /* Nobody signed in: the login screen, here and for as long as that lasts (clidialogs.js). */
    signInWatchInit();

    /* Last: everything above is the state being restored INTO, and must not be saved
       over the state it was restored FROM. */
    startViewStatePersistence();

    /* With "Enable Remote Control for all sessions" set, every conversation the view opened
       with becomes reachable from a phone too — not just the ones created afterwards
       (createTab handles those). Last, so it runs over the fully restored set rather
       than racing the restore. A no-op when the preference is off, which is default. */
    if (typeof autoEnableRemoteControlAll === 'function') autoEnableRemoteControlAll();
  } finally {
    /* The view does its load-time work (ClaudeGuiView.pageReady) only after this. */
    if (window._pageBooted) _pageBooted();
  }
}

ccBootIfReady();
