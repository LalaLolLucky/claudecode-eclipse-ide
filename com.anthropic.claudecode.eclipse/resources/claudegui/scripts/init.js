/* Boot sequence — the top-level statements that used to run inline, in their
   ORIGINAL relative order. Runs last, after every declaration file has loaded.
   buildActionsSlash stays last (it needs SLASH_COMMANDS). */

initModelConfig();
/* Before createTab: the workspace root has to exist for the first conversation to
   belong to, and #tabs renders only the ACTIVE root's tabs. */
initRoots();
/* Reopen the roots and conversations this workspace was last closed with; only when
   there is nothing to restore does the view open its usual single empty conversation. */
if (!restoreViewState()) createTab();
updateCtxChip();
setEffort(effortIdx);
updateThinkingCheck();

/* Populate the actions-menu slash list (needs SLASH_COMMANDS from slash.js). */
buildActionsSlash();

/* Last: everything above is the state being restored INTO, and must not be saved
   over the state it was restored FROM. */
startViewStatePersistence();
