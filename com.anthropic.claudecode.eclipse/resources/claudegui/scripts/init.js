/* Boot sequence — the top-level statements that used to run inline, in their
   ORIGINAL relative order. Runs last, after every declaration file has loaded.
   buildActionsSlash stays last (it needs SLASH_COMMANDS). */

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

/* Populate the actions-menu slash list (needs SLASH_COMMANDS from slash.js). */
buildActionsSlash();

/* Last: everything above is the state being restored INTO, and must not be saved
   over the state it was restored FROM. */
startViewStatePersistence();

/* With "Enable remote control on startup" set, every conversation the view opened
   with becomes reachable from a phone too — not just the ones created afterwards
   (createTab handles those). Last, so it runs over the fully restored set rather
   than racing the restore. A no-op when the preference is off, which is default. */
if (typeof autoEnableRemoteControlAll === 'function') autoEnableRemoteControlAll();
