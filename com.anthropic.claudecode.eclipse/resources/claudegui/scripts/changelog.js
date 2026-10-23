/* changelog.js — the Changelog window the command menu opens (Support › Changelog): the
   notes of every version of this plugin, from its own CHANGELOG.md (ClaudeGuiView#changelogText,
   asked through _ide 'changelog'), opened on the installed version.

   A rail of versions down the left, newest at the top, grouped by minor version, the
   installed one marked; on the right the notes of the one picked, under a row of chips that
   count what it added, changed and fixed (a chip jumps to its section). The search box looks
   through every version: the rail keeps the versions that mention it, with how often, and in
   the notes the matches are marked and the lines without one dimmed. ↑/↓ walk the rail.

   The menu row says "New" until this window has been opened on the installed version. That
   is remembered in localStorage: a per-machine nicety, and losing it only shows the pill again. */

const CHANGELOG_SEEN_KEY = 'claude-changelog-seen';
/** The parsed changelog, once it has been read; null until then (and after a failed read,
    so a later opening tries again). */
let changelogCache = null;

/** This plugin's version, as the menu's version footer has it (controls.js). */
function installedPluginVersion() {
  if (pluginVersion === null) {
    try { pluginVersion = (window._pluginVersion && _pluginVersion()) || ''; } catch (e) { pluginVersion = ''; }
  }
  return pluginVersion;
}
function loadChangelog() {
  if (changelogCache) return changelogCache;
  const res = ideErrand('changelog');
  if (!res.startsWith('ok:')) return [];
  changelogCache = parseChangelog(res.slice(3));
  return changelogCache;
}
/** Whether the menu row should say "New": this version has notes, and they have not been
    opened on this machine. */
function changelogUnseen() {
  const v = installedPluginVersion();
  let seen = null;
  try { seen = localStorage.getItem(CHANGELOG_SEEN_KEY); } catch (e) { return false; }
  if (!v || seen === v) return false;
  return loadChangelog().some(e => e.version === v);
}
function markChangelogSeen() {
  const v = installedPluginVersion();
  if (v) try { localStorage.setItem(CHANGELOG_SEEN_KEY, v); } catch (e) {}
}

/* ===================== Reading CHANGELOG.md ===================== */
/* An entry is "## [version] — date", maybe with "*(current)*" after it; its sections are
   "### Added", "### Fixed", …; entries are divided by "---". */
const CL_ENTRY = /^## \[([^\]]+)\]\s*(?:[—–-]\s*(\d{4}-\d{2}-\d{2}))?/;
const CL_SECTION = /^###\s+(.+?)\s*$/;

function parseChangelog(text) {
  const entries = [];
  let cur = null, sec = null;
  String(text).replace(/\r\n?/g, '\n').split('\n').forEach(line => {
    const h = CL_ENTRY.exec(line);
    if (h) {
      cur = { version: h[1].trim(), date: h[2] || '', intro: [], sections: [] };
      sec = null;
      entries.push(cur);
      return;
    }
    if (!cur) return;   // the file's own heading and introduction
    const s = CL_SECTION.exec(line);
    if (s) {
      sec = { title: s[1], kind: clKind(s[1]), lines: [] };
      cur.sections.push(sec);
      return;
    }
    (sec ? sec.lines : cur.intro).push(line);
  });
  entries.forEach(e => {
    e.intro = clTrim(e.intro);
    e.sections.forEach(s => {
      s.count = s.lines.filter(l => /^[-*+] /.test(l)).length;
      s.md = clTrim(s.lines);
      delete s.lines;
    });
    e.sections = e.sections.filter(s => s.md);
    // What the search looks through: the words, not where the links go.
    e.text = [e.version, e.intro].concat(e.sections.map(s => s.title + '\n' + s.md)).join('\n')
      .replace(/\]\([^)]*\)/g, ']').toLowerCase();
  });
  return entries;
}
/** The lines of a part, without the blank lines and the "---" divider at its ends. */
function clTrim(lines) {
  const l = lines.slice();
  while (l.length && (!l[l.length - 1].trim() || /^-{3,}\s*$/.test(l[l.length - 1]))) l.pop();
  while (l.length && !l[0].trim()) l.shift();
  return l.join('\n');
}
/** The colour a section is drawn in, by the first word of its heading. */
function clKind(title) {
  const w = title.toLowerCase();
  for (const k of ['added', 'changed', 'fixed', 'removed', 'security', 'deprecated']) if (w.startsWith(k)) return k;
  if (w.startsWith('known')) return 'known';
  return 'other';
}
function clDay(iso) {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(iso || '');
  return m ? new Date(+m[1], +m[2] - 1, +m[3]) : null;
}
function clDate(iso, withYear) {
  const d = clDay(iso);
  if (!d) return '';
  return d.toLocaleDateString(undefined, withYear ? { year: 'numeric', month: 'short', day: 'numeric' }
                                                  : { month: 'short', day: 'numeric' });
}
/** "3.2" of "3.2.23", the rail's groups. */
function clLine(version) {
  const m = /^(\d+)\.(\d+)/.exec(version);
  return m ? m[1] + '.' + m[2] : version;
}
function clHits(e, q) {
  if (!q) return 0;
  let n = 0, at = 0;
  while ((at = e.text.indexOf(q, at)) >= 0) { n++; at += q.length; }
  return n;
}

/* ===================== The window ===================== */
function openChangelogDialog() {
  const all = loadChangelog();
  const installed = installedPluginVersion();
  const start = Math.max(0, all.findIndex(e => e.version === installed));
  const dlg = {
    kind: 'changelog', all, installed, at: start, query: '', els: null,
    draw() {
      const win = document.getElementById('cli-win');
      win.classList.add('cl');
      const body = cwFrame('Changelog', null, true);
      body.classList.add('cl-body');
      if (!all.length) {
        win.classList.remove('cl');
        body.classList.remove('cl-body');
        body.appendChild(cwEl('div', 'cw-dim', "This build doesn't carry the changelog."));
        return;
      }
      // Over everything: a newer version of this plugin than the one these notes are of.
      const update = cwEl('div', 'cl-update-slot');
      body.appendChild(update);
      const bar = cwEl('div', 'cl-bar');
      const input = cwEl('input', 'cw-input cl-search');
      input.type = 'text'; input.placeholder = 'Search'; input.spellcheck = false;
      input.value = dlg.query;
      input.oninput = () => dlg.setQuery(input.value);
      bar.appendChild(input);
      body.appendChild(bar);
      const main = cwEl('div', 'cl-main');
      const rail = cwEl('div', 'cl-rail');
      const entry = cwEl('div', 'cl-entry');
      main.appendChild(rail); main.appendChild(entry);
      body.appendChild(main);
      document.getElementById('cli-win').appendChild(
        cwKeys([[['↑', '↓'], 'to browse versions'], [['Esc'], dlg.query ? 'to clear the search' : 'to close']]));
      dlg.els = { input, rail, entry, update };
      dlg.paintUpdate();
      dlg.paintRail();
      dlg.paintEntry();
      setTimeout(() => { if (cliDlg === dlg) input.focus(); }, 0);
    },
    close() { document.getElementById('cli-win').classList.remove('cl'); },
    escape() {
      if (!dlg.query) return false;
      dlg.els.input.value = '';
      dlg.setQuery('');
      return true;
    },
    key(e) {
      if (!dlg.els) return false;
      const k = e.key;
      if (k !== 'ArrowDown' && k !== 'ArrowUp' && k !== 'PageDown' && k !== 'PageUp') return false;
      const shown = dlg.shown();
      if (!shown.length) return true;
      const i = shown.indexOf(dlg.at);
      const step = (k === 'PageDown' || k === 'PageUp') ? 10 : 1;
      const down = k === 'ArrowDown' || k === 'PageDown';
      const to = i < 0 ? 0 : Math.min(shown.length - 1, Math.max(0, i + (down ? step : -step)));
      dlg.pick(shown[to]);
      return true;
    },
    /** The header saying a newer version is out, when the last look for updates found one
        (updates.js; it is asked for when this window opens, and fills this in when it answers). */
    paintUpdate() {
      if (!dlg.els) return;
      const slot = dlg.els.update;
      slot.innerHTML = '';
      const pl = typeof lastUpdateCheck !== 'undefined' && lastUpdateCheck && lastUpdateCheck.plugin;
      if (!pl || !pl.updateAvailable) return;
      const head = cwEl('div', 'cl-update');
      head.appendChild(cwEl('span', 'cl-update-text',
        'Version ' + pl.latest + ' is available — you have ' + pl.installed + '.'));
      head.appendChild(cwBtn('Update…', 'primary', () => openUpdatesDialog()));
      if (pl.url) head.appendChild(cwBtn('Release notes', '', () => { if (window._openExternal) _openExternal(pl.url); }));
      slot.appendChild(head);
    },
    /** The versions on the rail: all of them, or those that mention the search. */
    shown() {
      const q = dlg.query;
      return all.map((e, i) => i).filter(i => !q || clHits(all[i], q) > 0);
    },
    setQuery(raw) {
      const q = String(raw || '').trim().toLowerCase();
      if (q === dlg.query) return;
      const had = !!dlg.query;
      dlg.query = q;
      const shown = dlg.shown();
      if (shown.length && shown.indexOf(dlg.at) < 0) dlg.at = shown[0];
      if (had !== !!q) {   // the footer says what Esc does now
        const keys = document.querySelector('#cli-win .cw-keys');
        if (keys) keys.replaceWith(cwKeys([[['↑', '↓'], 'to browse versions'],
                                           [['Esc'], q ? 'to clear the search' : 'to close']]));
      }
      dlg.paintRail();
      dlg.paintEntry();
    },
    pick(i) {
      if (i === dlg.at) return;
      dlg.at = i;
      dlg.els.rail.querySelectorAll('.cl-ver').forEach(r => r.classList.toggle('sel', +r.dataset.i === i));
      const row = dlg.els.rail.querySelector('.cl-ver.sel');
      if (row && row.scrollIntoView) row.scrollIntoView({ block: 'nearest' });
      dlg.paintEntry();
    },
    paintRail() {
      const { rail } = dlg.els;
      const q = dlg.query;
      const shown = dlg.shown();
      rail.innerHTML = '';
      let line = null;
      shown.forEach(i => {
        const e = all[i];
        if (clLine(e.version) !== line) {
          line = clLine(e.version);
          rail.appendChild(cwEl('div', 'cl-line', 'v' + line));
        }
        const row = cwEl('div', 'cl-ver' + (i === dlg.at ? ' sel' : '') + (e.version === installed ? ' installed' : ''));
        row.dataset.i = i;
        row.appendChild(cwEl('span', 'cl-dot'));
        row.appendChild(cwEl('span', 'cl-v', e.version));
        const hits = clHits(e, q);
        row.appendChild(hits ? cwEl('span', 'cl-hits', String(hits)) : cwEl('span', 'cl-d', clDate(e.date, false)));
        row.title = e.version + (e.date ? ' — ' + clDate(e.date, true) : '') + (e.version === installed ? ' (installed)' : '');
        row.onclick = () => dlg.pick(i);
        rail.appendChild(row);
      });
      if (!shown.length) rail.appendChild(cwEl('div', 'cw-dim cl-none', 'No version mentions “' + q + '”.'));
      const sel = rail.querySelector('.cl-ver.sel');
      if (sel && sel.scrollIntoView) sel.scrollIntoView({ block: 'nearest' });
    },
    paintEntry() {
      const { entry } = dlg.els;
      entry.innerHTML = '';
      entry.scrollTop = 0;
      const shown = dlg.shown();
      if (!shown.length) return;
      const e = all[dlg.at];

      const top = cwEl('div', 'cl-top');
      const name = cwEl('div', 'cl-name');
      name.appendChild(cwEl('span', 'cl-vname', 'v' + e.version));
      if (e.version === installed) name.appendChild(cwEl('span', 'cl-tag installed', 'Installed'));
      if (dlg.at === 0 && e.version !== installed) name.appendChild(cwEl('span', 'cl-tag latest', 'Latest'));
      top.appendChild(name);
      const day = clDay(e.date);
      if (day) top.appendChild(cwEl('div', 'cl-when', clDate(e.date, true) + ' · ' + cwAgo(day.getTime())));
      entry.appendChild(top);

      const counted = e.sections.filter(s => s.count);
      if (counted.length) {
        const chips = cwEl('div', 'cl-chips');
        counted.forEach(s => {
          const chip = cwEl('span', 'cl-chip k-' + s.kind);
          chip.appendChild(cwEl('b', '', String(s.count)));
          chip.appendChild(document.createTextNode(' ' + s.title));
          chip.title = 'Go to ' + s.title;
          chip.onclick = () => {   // .cl-entry is the sections' offsetParent (dialogs.css)
            const at = entry.querySelector('[data-sec="' + e.sections.indexOf(s) + '"]');
            if (at) entry.scrollTop = at.offsetTop - 4;
          };
          chips.appendChild(chip);
        });
        entry.appendChild(chips);
      }

      if (e.intro) {
        const intro = cwEl('div', 'a-body cl-md');
        intro.innerHTML = renderMarkdown(e.intro);
        entry.appendChild(intro);
      }
      e.sections.forEach((s, k) => {
        const part = cwEl('div', 'cl-sec k-' + s.kind);
        part.dataset.sec = k;
        part.appendChild(cwEl('div', 'cl-sec-h', s.title));
        const md = cwEl('div', 'a-body cl-md');
        md.innerHTML = renderMarkdown(s.md);
        part.appendChild(md);
        entry.appendChild(part);
      });

      const at = shown.indexOf(dlg.at);
      const older = shown[at + 1], newer = at > 0 ? shown[at - 1] : undefined;
      if (older !== undefined || newer !== undefined) {
        const nav = cwEl('div', 'cl-nav');
        const step = (i, label) => {
          const b = cwEl('button', 'cl-step', label);
          b.type = 'button';
          b.onclick = () => dlg.pick(i);
          return b;
        };
        nav.appendChild(older !== undefined ? step(older, '← v' + all[older].version) : cwEl('span'));
        nav.appendChild(newer !== undefined ? step(newer, 'v' + all[newer].version + ' →') : cwEl('span'));
        entry.appendChild(nav);
      }
      if (dlg.query) clMark(entry, dlg.query);
    },
  };
  openCliDialog(dlg);
  // Whether a newer version is out is not known until it has been looked for. Asked once,
  // here, when it has not been: the answer fills the header in, and nothing waits for it.
  if (typeof lastUpdateCheck !== 'undefined' && !lastUpdateCheck && window._checkUpdates) {
    try { _checkUpdates(); } catch (e) {}
  }
  if (all.length) markChangelogSeen();
}

/** Marks every match of `q` in the notes, dims the lines of a list that have none, and
    brings the first match into view. */
function clMark(entry, q) {
  entry.querySelectorAll('.cl-md li').forEach(li => {
    if (!li.textContent.toLowerCase().includes(q)) li.classList.add('cl-dim');
  });
  const walker = document.createTreeWalker(entry, NodeFilter.SHOW_TEXT);
  const nodes = [];
  while (walker.nextNode()) {
    const n = walker.currentNode;
    if (n.nodeValue.toLowerCase().includes(q) && !n.parentNode.closest('.cl-nav')) nodes.push(n);
  }
  nodes.forEach(n => {
    const text = n.nodeValue, low = text.toLowerCase();
    const frag = document.createDocumentFragment();
    let from = 0, at;
    while ((at = low.indexOf(q, from)) >= 0) {
      if (at > from) frag.appendChild(document.createTextNode(text.slice(from, at)));
      frag.appendChild(cwEl('mark', 'cl-hit', text.slice(at, at + q.length)));
      from = at + q.length;
    }
    if (from < text.length) frag.appendChild(document.createTextNode(text.slice(from)));
    n.parentNode.replaceChild(frag, n);
  });
  const first = entry.querySelector('.cl-hit');
  if (first) {
    const box = entry.getBoundingClientRect(), r = first.getBoundingClientRect();
    if (r.bottom > box.bottom) entry.scrollTop += r.top - box.top - 40;
  }
}
