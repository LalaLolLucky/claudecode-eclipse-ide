/* cliversion.js — reconciles the INSTALLED Claude Code CLI with the latest release.

   The model chooser is built from the ACCOUNT's available models (/v1/models),
   which says nothing about what the installed binary understands: a CLI older
   than a model release rejects it, and because aliases ("opus"/"sonnet") resolve
   inside the CLI, an old binary silently resolves them to an OLDER model rather
   than erroring. Rather than trying to enumerate what the binary supports (the
   ids are compiled into a ~265MB executable and scraping them yields false
   positives), we surface the version: a dismissible banner when an update is
   available, and a hint appended to any model-rejection error. */

/** {installed, latest, updateAvailable} — pushed by Java (onCliVersion). */
let cliVersion = null;

window.onCliVersion = function (json) {
  let info;
  try { info = JSON.parse(json); } catch (e) { return; }
  if (!info) return;
  cliVersion = info;
  renderUpdateBanner();
};

/* FreeBSD setup guide (Markdown), pushed by Java while the `claude` CLI is missing.
   Shown once per tab pane as a card in the conversation; re-pushes (page reload,
   version re-check) replace it rather than stacking a second copy.

   With nothing to talk to, the view becomes the guide: body.setup-guide-mode hides
   the composer and the root-directory rows (layout.css), and the tab holding the
   guide loses its rename/close actions and cannot be closed (tabs.js). Java
   disables New Session and Session history on the view toolbar at the same time. */
let setupGuideMode = false;

window.onSetupGuide = function (md) {
  const t = activeTab();
  const pane = t ? t.pane : messagesEl;
  if (!pane || !md) return;
  setupGuideMode = true;
  document.body.classList.add('setup-guide-mode');
  if (t) { t.setupGuide = true; renderTabs(); }
  clearWelcome(pane);
  const old = pane.querySelector('.setup-guide');
  if (old) old.remove();
  const card = document.createElement('div');
  card.className = 'turn setup-guide';
  card.innerHTML = '<div class="sg-body a-body"></div>';
  card.querySelector('.sg-body').innerHTML = renderMarkdown(md);
  pane.appendChild(card);
};

/* Dismissal is per panel-load only (no pref): the banner shouldn't nag within a
   session, but a genuinely outdated CLI is worth re-surfacing next time. */
let updateBannerDismissed = false;

/* null once an update has been started here: 'running' | 'done' | 'failed'.
   From that point the banner is a RESULT notice and belongs to the user — it
   stays until they close it. Without this the version re-check that follows a
   successful update reports "up to date" and hides the banner immediately,
   taking the "restart Eclipse" instruction with it. */
let updateRunState = null;

function renderUpdateBanner() {
  const bar = document.getElementById('update-banner');
  if (!bar) return;
  if (updateBannerDismissed) { bar.classList.remove('show'); return; }
  // Sticky: keep whatever text/buttons the run left in place, and never re-hide.
  if (updateRunState) { bar.classList.add('show'); return; }
  const show = !!(cliVersion && cliVersion.updateAvailable);
  bar.classList.toggle('show', show);
  if (!show) return;
  const txt = bar.querySelector('.ub-text');
  if (txt) {
    txt.textContent = 'Claude Code ' + cliVersion.latest + ' is available (you have '
                    + cliVersion.installed + '). Update to use the newest models.';
  }
}

function dismissUpdateBanner() {
  updateBannerDismissed = true;
  renderUpdateBanner();
}

/* Set once `claude update` reports it deferred to a package manager instead of updating
   directly (onCliUpdateDone below) — the exact follow-up command it printed, e.g. "brew
   upgrade claude-code". While set, the banner's button runs THAT instead of `claude
   update` again, which would just print the same advice a second time. */
let pendingManagedCommand = null;

/** Runs `claude update` (the CLI's own updater — install-method agnostic), or, once a
 *  prior run deferred to a package manager, that manager's own update command instead. */
function runCliUpdate() {
  const bar = document.getElementById('update-banner');
  if (!bar) return;
  updateRunState = 'running';   // from here the banner is ours until dismissed
  const btn = bar.querySelector('.ub-btn');
  const txt = bar.querySelector('.ub-text');
  if (pendingManagedCommand) {
    const cmd = pendingManagedCommand;
    if (btn) { btn.classList.add('busy'); btn.textContent = 'Running…'; }
    if (txt) txt.textContent = 'Running ' + cmd + ' — this can take a minute.';
    try { if (window._runManagedUpdate) window._runManagedUpdate(cmd); } catch (e) {}
    return;
  }
  if (btn) { btn.classList.add('busy'); btn.textContent = 'Updating…'; }
  if (txt) txt.textContent = 'Running claude update — this can take a minute.';
  try { if (window._updateCli) window._updateCli(); } catch (e) {}
}

window.onCliUpdateDone = function (json) {
  let res;
  try { res = JSON.parse(json); } catch (e) { res = null; }
  const bar = document.getElementById('update-banner');
  if (!bar) return;
  const btn = bar.querySelector('.ub-btn');
  const txt = bar.querySelector('.ub-text');
  // "claude update" defers entirely to the system package manager when one owns the
  // install (Homebrew, apt, ...): it prints manual instructions and does no actual
  // update, but apparently still exits 0 (it isn't an error, just advice) — so `res.ok`
  // alone can't tell "updated" from "told you to go run brew yourself". Detected by the
  // CLI's own "is managed by" phrasing rather than trusting the exit code. Guarded on
  // !pendingManagedCommand so this only ever fires off `claude update`'s OWN output, not
  // a second time off whatever the follow-up command itself printed.
  if (!pendingManagedCommand && res && res.output && /is managed by/i.test(res.output)) {
    const lines = String(res.output).split('\n').map(l => l.trim()).filter(Boolean);
    const howToIdx = lines.findIndex(l => /^to update/i.test(l));
    const cmd = howToIdx >= 0 ? lines[howToIdx + 1] : null;
    updateRunState = 'failed';   // not really a failure, but keeps the banner from being swept as "done"
    if (cmd) {
      pendingManagedCommand = cmd;
      if (btn) { btn.classList.remove('busy'); btn.textContent = 'Run: ' + cmd; }
    } else if (btn) {
      btn.classList.remove('busy'); btn.style.display = 'none';   // nothing parsed to run
    }
    if (txt) txt.textContent = lines.find(l => /is managed by/i.test(l))
      || 'Claude is managed by your system package manager.';
    return;
  }
  pendingManagedCommand = null;
  if (res && res.ok) {
    // Restart matters: long-lived per-tab processes keep running the OLD binary.
    // Stays on screen (updateRunState) until the user closes it, so this doesn't
    // flash past when the follow-up version check says "up to date".
    updateRunState = 'done';
    if (btn) { btn.classList.remove('busy'); btn.style.display = 'none'; }
    if (txt) txt.textContent = 'Claude Code updated. Restart Eclipse (or open a new tab) to use it.';
    return;
  }
  updateRunState = 'failed';
  if (btn) { btn.classList.remove('busy'); btn.textContent = 'Retry'; }
  if (txt) {
    const detail = (res && res.output) ? (' — ' + String(res.output).split('\n')[0]) : '';
    txt.textContent = 'Update failed' + detail + '. You can also run "claude update" in a terminal.';
  }
};

/** Model ids the CLI rejects produce an error naming the model; add the version hint. */
function augmentError(msg) {
  const m = String(msg || '');
  if (!cliVersion || !cliVersion.installed) return m;
  // Only annotate errors that actually look like a model rejection.
  if (!/model/i.test(m)) return m;
  if (!/(unknown|not (a )?(valid|supported|recognized|found))|invalid|unsupported/i.test(m)) return m;
  let hint = ' (Your installed Claude Code is ' + cliVersion.installed;
  hint += (cliVersion.updateAvailable && cliVersion.latest)
        ? '; ' + cliVersion.latest + ' is available — updating may add this model.)'
        : '. This model may need a newer Claude Code.)';
  return m + hint;
}
