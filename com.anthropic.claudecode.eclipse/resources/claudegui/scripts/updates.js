/* updates.js — the window the command menu opens (Support › Check for updates): whether a newer
   Claude Code CLI and a newer version of this plugin have been published.

   The question is asked of Java afresh each time the window opens (_checkUpdates; the CLI's
   hour-long cache is dropped) and answered on window.onUpdateCheck. The CLI is then updated
   the way the update banner does it (cliversion.js, runCliUpdate), and the plugin through
   Eclipse's own update wizard: this only finds out. */

/** The last answer, {cli: {installed, latest, updateAvailable}, plugin: {installed, latest,
    updateAvailable, url}}; null until the first check has come back. */
let lastUpdateCheck = null;

/** Whether the last look found anything newer, for the menu row's "New" pill. The CLI half is
    also known without a check (the banner's own, at startup), so that counts too. */
function updateKnown() {
  if (lastUpdateCheck) return !!(lastUpdateCheck.cli.updateAvailable || lastUpdateCheck.plugin.updateAvailable);
  return !!(typeof cliVersion !== 'undefined' && cliVersion && cliVersion.updateAvailable);
}

window.onUpdateCheck = function (json) {
  let res;
  try { res = JSON.parse(json); } catch (e) { return; }
  if (!res || !res.cli || !res.plugin) return;
  lastUpdateCheck = res;
  if (cliDlg && cliDlg.kind === 'updates') { cliDlg.checking = false; cliDlg.draw(); }
  else if (cliDlg && cliDlg.kind === 'changelog') cliDlg.paintUpdate();
};

/* What came of "Update plugin…" (PluginUpdater): the wizard opened for just this plugin, or why
   it did not. The release page stands in for a Eclipse that cannot do it. */
window.onPluginUpdate = function (json) {
  let res;
  try { res = JSON.parse(json); } catch (e) { return; }
  const dlg = cliDlg && cliDlg.kind === 'updates' ? cliDlg : null;
  if (!dlg) return;
  dlg.updating = false;
  const status = res && res.status;
  if (status === 'wizard') { closeCliDialog(); return; }
  const pl = lastUpdateCheck && lastUpdateCheck.plugin;
  if (status === 'unavailable') {
    dlg.note = "This Eclipse can't update plugins from here; the release page is open instead.";
    if (pl && pl.url && window._openExternal) _openExternal(pl.url);
  } else if (status === 'notinstalled') {
    dlg.note = "This copy of the plugin wasn't installed from an update site (it may be running from a workspace), so Eclipse can't update it.";
  } else if (status === 'nothing') {
    dlg.note = "Eclipse found nothing newer on the plugin's update site yet. It can lag a release by a minute or two.";
  } else {
    dlg.note = "Couldn't look for the update" + (res && res.message ? ': ' + res.message : '.');
  }
  dlg.draw();
};

/** One line of the window: what it is about, how it stands, and what can be done about it. */
function updateRow(body, name, verdict, tone, buttons) {
  body.appendChild(cwEl('div', 'cw-sec', name));
  body.appendChild(cwEl('div', tone || 'cw-text', verdict));
  buttons.forEach(b => body.appendChild(cwBtn(b.label, b.primary ? 'primary' : '', b.run)));
}

function openUpdatesDialog() {
  const dlg = {
    kind: 'updates', checking: false, updating: false, note: null,
    check() {
      dlg.checking = true; dlg.note = null;
      dlg.draw();
      if (window._checkUpdates) { try { _checkUpdates(); return; } catch (e) {} }
      dlg.checking = false;   // a build without the errand: nothing to ask
      lastUpdateCheck = null;
      dlg.draw();
    },
    draw() {
      const body = cwFrame('Check for updates', null, false);
      const r = lastUpdateCheck;
      if (dlg.checking) {
        body.appendChild(cwEl('div', 'cw-dim', 'Checking…'));
      } else if (!r) {
        body.appendChild(cwEl('div', 'cw-msg', "This build can't check for updates."));
      } else {
        const cli = r.cli, pl = r.plugin;
        // The CLI.
        if (!cli.installed) {
          updateRow(body, 'Claude Code CLI', "Claude Code isn't installed, or couldn't be run.", 'cw-warn', []);
        } else if (!cli.latest) {
          updateRow(body, 'Claude Code CLI', 'v' + cli.installed + " — couldn't look up the latest version. Are you offline?", 'cw-warn', []);
        } else if (cli.updateAvailable) {
          updateRow(body, 'Claude Code CLI', 'v' + cli.latest + ' is available (you have v' + cli.installed + ').', 'cw-text', [
            { label: 'Update Claude Code', primary: true, run: () => {
              // The banner runs it and shows how it went, as it does from its own button.
              closeCliDialog();
              updateBannerDismissed = false;
              runCliUpdate();
              renderUpdateBanner();
            } },
          ]);
        } else {
          updateRow(body, 'Claude Code CLI', 'v' + cli.installed + ' · up to date', 'cw-ok', []);
        }
        // The plugin.
        if (!pl.installed) {
          updateRow(body, 'Plugin', "Couldn't read which version of the plugin is installed.", 'cw-warn', []);
        } else if (!pl.latest) {
          updateRow(body, 'Plugin', 'v' + pl.installed + " — couldn't look up the latest release. Are you offline?", 'cw-warn', []);
        } else if (pl.updateAvailable) {
          const open = () => { if (pl.url && window._openExternal) _openExternal(pl.url); };
          updateRow(body, 'Plugin', 'v' + pl.latest + ' is available (you have v' + pl.installed + ').', 'cw-text', [
            { label: dlg.updating ? 'Looking for the update…' : 'Update plugin…', primary: true, run: () => {
              // Eclipse's update wizard, for this plugin alone and from its own site (PluginUpdater);
              // the answer comes on onPluginUpdate.
              if (dlg.updating) return;
              if (!window._updatePlugin) { open(); return; }
              dlg.updating = true; dlg.note = null; dlg.draw();
              try { _updatePlugin(); } catch (e) { dlg.updating = false; open(); dlg.draw(); }
            } },
            { label: 'Release notes', run: open },
          ]);
        } else {
          updateRow(body, 'Plugin', 'v' + pl.installed + ' · up to date', 'cw-ok', []);
        }
      }
      if (dlg.note) {
        const note = cwEl('div', 'cw-warn', dlg.note);
        note.style.marginTop = '10px';
        body.appendChild(note);
      }
      const row = cwEl('div', 'cw-confirm');
      const again = cwBtn('Check again', '', () => { if (!dlg.checking) dlg.check(); });
      again.disabled = dlg.checking;
      row.appendChild(again);
      row.appendChild(cwBtn('Close', '', closeCliDialog));
      body.appendChild(row);
    },
  };
  openCliDialog(dlg);
  dlg.check();
}
