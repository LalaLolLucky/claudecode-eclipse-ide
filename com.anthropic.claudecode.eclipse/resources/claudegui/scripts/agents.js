/* agents.js — "Agents" popup: lists Agent tool calls visible in the CURRENT tab's own
   transcript, with a status dot per agent (running/done/error). Reads the rendered DOM
   directly (collectAgents below) rather than a separately tracked list fed only by live
   streaming events — that earlier approach went empty on any restart (a fresh JS runtime
   starts with an empty Map) and would have gone empty on a reload/resume too, or mixed
   agents across tabs, since it wasn't scoped per conversation. Reading the transcript that's
   already there sidesteps all three: it's exactly as fresh as the tool cards themselves,
   whether they got rendered by live streaming (chat.js's addToolLine) or by loading history
   (history.js calling makeToolLine directly) — same markup either way. Same modal-overlay
   idiom as #rewind-overlay/#account-overlay (scrim + centered card), not a corner dropdown —
   opened via the /agents slash command (slash.js); no toolbar entry point yet. */

const agentsOverlayEl = document.getElementById('agents-overlay');
const agentsWinEl = document.getElementById('agents-win');

/** Scans the active tab's own pane for Agent tool-line cards and reads their current state
 *  straight off the rendered markup — the same .tname/.tagent-type/.tdesc/.dot a person
 *  looking at the transcript itself would see, so this can never drift out of sync with it. */
function collectAgents() {
  const pane = activeTab() ? activeTab().pane : null;
  if (!pane) return [];
  const agents = [];
  pane.querySelectorAll('.tool-line').forEach(line => {
    if (!AGENT_KEYS.has(line.dataset.tname)) return;
    const dot = line.querySelector('.dot');
    const status = dot && dot.classList.contains('done') ? 'done'
        : dot && dot.classList.contains('red') ? 'error' : 'running';
    const typeEl = line.querySelector('.tagent-type');
    const descEl = line.querySelector('.tdesc');
    const type = typeEl ? typeEl.textContent.replace(/^\(|\)$/g, '') : '';
    const description = (descEl && descEl.textContent) || 'Agent';
    agents.push({ type, description, status });
  });
  return agents;
}

function renderAgentsWin() {
  agentsWinEl.innerHTML = '';
  const head = document.createElement('div'); head.className = 'ap-head';
  const title = document.createElement('span'); title.className = 'ap-title'; title.textContent = 'Agents';
  const x = document.createElement('span'); x.className = 'ap-x'; x.innerHTML = ICONS.X;
  x.onclick = () => closeAgentsPanel();
  head.appendChild(title); head.appendChild(x);
  agentsWinEl.appendChild(head);

  const list = document.createElement('div'); list.className = 'ap-list';
  const agents = collectAgents();
  if (!agents.length) {
    const empty = document.createElement('div'); empty.className = 'ap-empty';
    empty.textContent = 'No agents in this conversation yet.';
    list.appendChild(empty);
  } else {
    // Most recent first — the one you just kicked off is what you're watching.
    agents.reverse().forEach(a => {
      const row = document.createElement('div'); row.className = 'ap-row';
      const dot = document.createElement('span');
      dot.className = 'dot ' + (a.status === 'done' ? 'done' : a.status === 'error' ? 'red' : 'spin');
      const label = document.createElement('span'); label.className = 'ap-label';
      label.textContent = a.type ? '(' + a.type + ') ' + a.description : a.description;
      row.appendChild(dot); row.appendChild(label);
      list.appendChild(row);
    });
  }
  agentsWinEl.appendChild(list);
}

/* Re-renders in place while open — called by chat.js whenever the registry changes (a new
   Agent call starts, or one finishes). A no-op while closed; opening always renders fresh. */
function renderAgentsPanel() {
  if (!agentsOverlayEl.classList.contains('open')) return;
  renderAgentsWin();
}

function openAgentsPanel() {
  renderAgentsWin();
  agentsOverlayEl.classList.add('open');
  document.addEventListener('keydown', onAgentsKey, true);
}
function closeAgentsPanel() {
  agentsOverlayEl.classList.remove('open');
  document.removeEventListener('keydown', onAgentsKey, true);
}
function toggleAgentsPanel() {
  if (agentsOverlayEl.classList.contains('open')) closeAgentsPanel(); else openAgentsPanel();
}
function onAgentsKey(e) {
  if (e.key === 'Escape') { e.preventDefault(); closeAgentsPanel(); }
}
// Same idiom as rewind.js: clicking the scrim itself (not the card) closes the popup.
agentsOverlayEl.addEventListener('click', (e) => {
  if (e.target.id === 'agents-overlay') closeAgentsPanel();
});
// Called from chat.js (addToolLine/applyToolResult) via window.renderAgentsPanel — the
// indirection lets chat.js fire this before agents.js has necessarily finished defining
// it on a very first paint, and lets it stay a no-op if this script somehow isn't loaded.
window.renderAgentsPanel = renderAgentsPanel;
window.toggleAgentsPanel = toggleAgentsPanel;
