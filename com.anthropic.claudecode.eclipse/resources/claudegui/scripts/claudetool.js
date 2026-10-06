/* claudetool.js — Page side of the claudeCodeEclipse MCP tool's claudeCodeView module:
   lists the conversation tabs, opens them (under a chosen folder) and closes them, sends a prompt into one,
   switches Remote Control on or off for one, and sets a tab's model/effort/thinking/
   permission mode. Java calls window.__ccTool(requestJson) and hands the returned JSON
   back to Claude (ClaudeGuiView#pageTool). */

(function () {
  /* A request that cannot be carried out. Thrown before anything is changed, so a
     refused call leaves every tab as it was. */
  function refuse(msg) { throw { refused: msg }; }

  function bypassAllowed() {
    try { return window._bypassModeAllowed ? !!_bypassModeAllowed() : true; } catch (e) { return true; }
  }
  /* The permission modes the Modes menu offers, read from the menu itself so there is
     no second list to keep in step with it. */
  function modeIds() {
    const ids = [];
    document.querySelectorAll('#modes-menu .item[data-mode]').forEach(el => {
      const id = el.getAttribute('data-mode');
      if (id !== 'bypassPermissions' || bypassAllowed()) ids.push(id);
    });
    return ids;
  }
  function selectableModels() { return MODELS.filter(m => !m.disabled); }
  function underRemoteControl(t) { return !!(t && (t.remoteControlUrl || t.rcConnecting)); }

  function remoteControlState(t) {
    if (t.rcDisconnecting) return 'disconnecting';
    if (t.rcConnecting) return 'connecting';
    return t.remoteControlUrl ? 'on' : 'off';
  }
  function describe(t) {
    const out = describeSettings(t);
    out.remoteControl = remoteControlState(t);
    if (out.remoteControl === 'on') out.remoteControlUrl = t.remoteControlUrl;
    return out;
  }
  function describeSettings(t) {
    return { id: t.id, title: t.title, active: t === activeTab(), root: rootPathOf(t),
      sessionId: t.sessionId || '', model: t.model || '', modelLabel: modelLabelFor(t.model || ''),
      effort: EFFORTS[t.effortIdx], thinking: !!t.thinking, mode: t.permMode || DEFAULT_PERM_MODE,
      streaming: !!t.streaming };
  }

  /* The settings a tab ends up with: `base` with the request laid over it, then made
     legal the way the composer's own controls keep it legal (see the thinking/effort
     gate in models.js). `t` is the tab being changed, null for one not created yet. */
  function resolve(req, base, t) {
    const out = { model: base.model, effortIdx: base.effortIdx, thinking: base.thinking,
                  permMode: base.permMode, notes: [] };
    if (req.model !== undefined) {
      let id = String(req.model);
      if (id.toLowerCase() === 'default') id = '';
      const entry = MODELS.find(m => m.id === id);
      if (entry ? entry.disabled : !/^claude-[\w.\-\[\]]+$/.test(id)) {
        refuse("Unknown model '" + req.model + "'. Use 'default', a full claude-… id, or one of: "
          + selectableModels().map(m => m.id).filter(Boolean).join(', ') + '.');
      }
      // The one model choice that can restart a conversation's process, which the view
      // confirms with the user first when other devices are attached to it.
      if (!id && base.model && underRemoteControl(t)) {
        refuse('Switching to the default model can restart this conversation, and it is under '
          + 'Remote Control. That switch has to be made in the view.');
      }
      out.model = id;
    }
    if (req.effort !== undefined) {
      const idx = EFFORTS.indexOf(String(req.effort).toLowerCase());
      if (idx < 0) refuse("Unknown effort '" + req.effort + "'. Use one of: " + EFFORTS.join(', ') + '.');
      out.effortIdx = idx;
    }
    if (req.thinking !== undefined) {
      if (typeof req.thinking !== 'boolean') refuse("'thinking' must be true or false.");
      out.thinking = req.thinking;
    }
    if (req.mode !== undefined) {
      const mode = String(req.mode);
      if (mode === 'bypassPermissions' && !bypassAllowed()) {
        refuse("Bypass permissions is turned off in this Eclipse's preferences.");
      }
      if (modeIds().indexOf(mode) < 0) {
        refuse("Unknown mode '" + req.mode + "'. Use one of: " + modeIds().join(', ') + '.');
      }
      out.permMode = mode;
    }

    if (underRemoteControl(t) && !out.thinking) {
      out.thinking = true;
      out.notes.push('Thinking stays on while Remote Control is active.');
    }
    if (isThinkingGatedModel(out.model) && !out.thinking
        && EFFORT_REQUIRES_THINKING.indexOf(EFFORTS[out.effortIdx]) >= 0) {
      const at = EFFORT_LABELS[out.effortIdx] + ' effort on ' + gateModelName(out.model);
      if (req.thinking === false && req.effort === undefined) {
        // Only thinking was asked for, so it is the effort that gives way.
        while (out.effortIdx > 0 && EFFORT_REQUIRES_THINKING.indexOf(EFFORTS[out.effortIdx]) >= 0) out.effortIdx--;
        out.notes.push('Effort lowered to ' + EFFORT_LABELS[out.effortIdx] + ': ' + at + ' needs thinking on.');
      } else {
        out.thinking = true;
        out.notes.push('Thinking turned on: required at ' + at + '.');
      }
    }
    return out;
  }

  function switchDividerName(id) {
    const entry = MODELS.find(m => m.id === id);
    return (entry && entry.fullId) ? entry.fullId : (/^claude-/.test(id) ? id : modelLabelFor(id));
  }

  /* Writes resolved settings to a tab the way a pick in the composer does: stored on
     the tab, saved for a resume, and handed to its running process. The composer is
     repainted only when the tab is the one in front; another tab picks them up from
     its own fields when it is switched to. */
  function apply(t, r) {
    const modelChanged = r.model !== t.model;
    t.model = r.model; t.effortIdx = r.effortIdx; t.thinking = r.thinking; t.permMode = r.permMode;
    if (modelChanged && !t.pane.querySelector('.welcome')) {
      t.pane.appendChild(makeSwitchDivider(switchDividerName(r.model)));
      if (t === activeTab()) scrollBottom();
    }
    if (t === activeTab()) applyTabSettings(t);
    persistTabPrefs(t);
    try {
      if (window._applySettings) _applySettings(t.id, t.permMode, EFFORTS[t.effortIdx], t.model, t.thinking ? '1' : '0');
    } catch (e) {}
  }

  /* The prompt of a request, trimmed. Slash commands are left to the composer: it runs
     some of them itself, against the tab in front, and none of that happens here. */
  function promptOf(req) {
    const text = (typeof req.prompt === 'string') ? req.prompt.trim() : '';
    if (!text) refuse("'prompt' is required: the message to send.");
    if (text.charAt(0) === '/') refuse('Slash commands are not sent through this tool; type them in the view.');
    return text;
  }

  /* Sends `text` as the user's next message in tab `t`, front or not — what doSend()
     does for the tab in front, without the composer: no draft, attachment or editor
     context goes with it, and the composer of the tab in front is left as it is.
     @returns {boolean} whether it was queued behind a turn already running */
  function send(t, text) {
    // A tab restored from the last Eclipse session holds only its session id until it
    // is first shown (see switchTab). Rebuilt here first, or that later rebuild would
    // empty the pane this turn is rendering into.
    if (t._restore) {
      const rs = t._restore;
      t._restore = null;
      loadHistory(rs.sessionId, rs.title, t);
    }
    t.cancelled = false;
    loadRender(t);
    const queueing = !!t.streaming;
    addUserMessage(text, null, [], null, nowIso(), t.pane, null);
    if (!queueing) { setStreaming(true); showWorking(); }
    else if (!workingEl) showWorking();
    if (window._sendToJava) {
      window._sendToJava(text, false, t.sessionId || '', t.permMode || DEFAULT_PERM_MODE, EFFORTS[t.effortIdx],
        t.model || '', t.thinking ? '1' : '0', t.id, '', rootPathOf(t));
    }
    persistTabPrefs(t);
    return queueing;
  }

  /* The tab an action that sends to or closes one is aimed at. No default: the tab in
     front is usually the one the call came from. */
  function tabForChange(req) {
    if (!req.tabId) refuse("'tabId' is required. Use action 'listTabs'.");
    const t = tabById(String(req.tabId));
    if (!t) refuse("No tab with id '" + req.tabId + "'. Use action 'listTabs'.");
    if (t.setupGuide) refuse('That tab is the setup guide, not a conversation.');
    return t;
  }

  /* Switches Remote Control on for `t` unless it already is, or is on its way — the
     same request /remote-control makes, for a tab that need not be in front. */
  function remoteControlOn(t) {
    if (t.remoteControlUrl || t.rcConnecting) return;
    beginConnecting(t);
    rcSend(t, true);
  }
  function requireRemoteControl() {
    if (!window._remoteControl) refuse('Remote Control is not available in this build.');
  }

  /* A message sent before the bridge is up never reaches the other devices, which is
     why the composer stays shut while a tab connects. A prompt for such a tab waits
     the same way, and goes out once the tab has stopped connecting — connected or
     not, since the conversation works here either way.
     @returns {boolean} whether the prompt is waiting rather than sent */
  function sendOnceConnected(t, text) {
    if (!t.rcConnecting) { send(t, text); return false; }
    const timer = setInterval(() => {
      if (!tabById(t.id)) { clearInterval(timer); return; }   // closed while it waited
      if (t.rcConnecting) return;
      clearInterval(timer);
      if (!t.rcDisconnecting) send(t, text);
    }, 250);
    return true;
  }

  /* Where a new conversation goes: `rootId` of a folder tab that is already open, or
     `path` of a folder that may be opened as one. Nothing is changed here, so a call
     refused further on leaves no folder tab behind. Without 'folder' it is the one in
     front.

     A folder Claude has not been run in before is not opened from here: the view asks
     "Trust this folder?" first, and that question is for whoever sits at the view, not
     for a tool call to answer or to leave waiting there. */
  function folderFor(req) {
    if (req.folder === undefined) return { rootId: activeRootId };
    const asked = (typeof req.folder === 'string') ? req.folder.trim() : '';
    // Java would read anything else against Eclipse's own working directory.
    if (!/^([A-Za-z]:[\\/]|[\\/])/.test(asked)) {
      refuse("'folder' must be the full path of a folder, such as a 'root' that 'listTabs' shows.");
    }
    // Same two looks as openRootDirectory: the path as given, then as Java spells it.
    let root = rootByPath(asked), path = asked;
    if (!root) {
      let info = null;
      try { info = window._folderInfo ? JSON.parse(_folderInfo(asked) || 'null') : null; } catch (e) {}
      if (!info) refuse('This view cannot look folders up, so a tab can only be opened in a folder that is already open.');
      path = info.path || asked;
      root = rootByPath(path);
      if (!root && !info.exists) refuse("There is no folder at '" + path + "'.");
      if (!root && !info.trusted) {
        refuse("Claude has not been run in '" + path + "' before, and starting it there needs the user's "
          + 'consent. They open that folder once in the Claude Code view ("New Claude root directory", or '
          + 'Open Claude Here) and answer "Trust this folder?"; a folder trusted in the Claude Terminal counts too.');
      }
    }
    // With the row hidden nothing shows which folder is in front, or leads back to another.
    if (rootDirectoriesRowHidden && !(root && root.id === activeRootId)) {
      refuse('The folder row is hidden in this Eclipse (preference "Hide the root directories row"), '
        + 'so a tab can only be opened in the folder in front.');
    }
    return root ? { rootId: root.id } : { path: path };
  }

  function result(t, notes) {
    const out = { ok: true, tab: describe(t) };
    if (notes.length) out.notes = notes;
    return out;
  }

  const ACTIONS = {
    listTabs: function () {
      return { ok: true, tabs: tabs.map(describe),
        models: selectableModels().map(m => ({ id: m.id, label: m.label })),
        efforts: EFFORTS.slice(), modes: modeIds() };
    },
    newTab: function (req) {
      const text = req.prompt !== undefined ? promptOf(req) : null;
      if (req.remoteControl !== undefined && typeof req.remoteControl !== 'boolean') {
        refuse("'remoteControl' must be true or false.");
      }
      if (req.remoteControl) requireRemoteControl();
      const r = resolve(req, { model: defaultModel(), effortIdx: DEFAULT_EFFORT_IDX,
        thinking: defaultThinking(), permMode: DEFAULT_PERM_MODE }, null);
      const where = folderFor(req);
      closeMenus();
      // A folder not open yet gets its tab from here, not from addRoot: that one would
      // be on the defaults and, with Remote Control on startup, already connecting.
      const rootId = where.rootId || addRoot(where.path, { select: false }).id;
      const t = createTab({ rootId: rootId, model: r.model, effortIdx: r.effortIdx,
        thinking: r.thinking, permMode: r.permMode });
      // After createTab: with Remote Control on startup set, it is already connecting.
      if (req.remoteControl) remoteControlOn(t);
      const waiting = text !== null && sendOnceConnected(t, text);
      const out = result(t, r.notes);
      if (waiting) out.promptDeferred = true;
      return out;
    },
    configureTab: function (req) {
      const t = req.tabId ? tabById(String(req.tabId)) : activeTab();
      if (!t) refuse(req.tabId ? "No tab with id '" + req.tabId + "'. Use action 'listTabs'." : 'No tab is open.');
      if (req.model === undefined && req.effort === undefined && req.thinking === undefined && req.mode === undefined) {
        refuse("Nothing to change: give at least one of 'model', 'effort', 'thinking', 'mode'.");
      }
      const r = resolve(req, { model: t.model || '', effortIdx: t.effortIdx, thinking: !!t.thinking,
        permMode: t.permMode || DEFAULT_PERM_MODE }, t);
      apply(t, r);
      return result(t, r.notes);
    },
    sendPrompt: function (req) {
      const t = tabForChange(req);
      const text = promptOf(req);
      if (t.rcConnecting || t.rcDisconnecting) refuse('That tab is switching Remote Control on or off; send once it has finished.');
      if (t.pendingCard) refuse('That tab is waiting for an answer to an approval or question card.');
      const queued = send(t, text);
      return { ok: true, queued: queued, tab: describe(t) };
    },
    remoteControl: function (req) {
      const t = tabForChange(req);
      if (typeof req.enabled !== 'boolean') refuse("'enabled' must be true or false.");
      requireRemoteControl();
      // One request at a time: the CLI's answer to a second one cannot be told from a
      // refusal of the first (see toggleRemoteControl).
      if (t.rcDisconnecting) refuse('That tab is still switching Remote Control off; try again in a moment.');
      if (req.enabled) remoteControlOn(t);
      else if (t.remoteControlUrl || t.rcConnecting) { beginDisconnecting(t); rcSend(t, false); }
      return result(t, []);
    },
    closeTab: function (req) {
      const t = tabForChange(req);
      // Its last conversation closing closes the folder too, and the last folder asks
      // the user before taking the whole view down; neither is this tool's to start.
      if (tabs.filter(x => x.rootId === t.rootId).length === 1) {
        refuse('That is the only conversation in its folder, so closing it would close the folder. Open another tab first.');
      }
      closeTab(t.id);
      return { ok: true, closed: t.id, tabs: tabs.map(describe) };
    }
  };

  window.__ccTool = function (json) {
    let out;
    try {
      const req = JSON.parse(json);
      const action = ACTIONS.hasOwnProperty(req.action) ? ACTIONS[req.action] : null;
      if (!action) refuse("Unknown action '" + req.action + "'. Use one of: " + Object.keys(ACTIONS).join(', ') + '.');
      out = action(req);
    } catch (e) {
      out = { ok: false, error: (e && e.refused) || ('claudeCodeView failed: ' + (e && e.message || e)) };
    }
    try { if (window.__ccDebug && window._debugLog) _debugLog('[CCTOOL] ' + json + ' -> ' + JSON.stringify(out)); } catch (e) {}
    return JSON.stringify(out);
  };
})();
