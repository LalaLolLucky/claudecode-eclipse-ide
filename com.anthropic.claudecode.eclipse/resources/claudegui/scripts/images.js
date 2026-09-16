/* images.js — pasted-image attachments for the composer, VSCode-style
   (joebiden4): a screenshot/image paste (Ctrl+V) into the input becomes a CHIP
   — tiny thumbnail + "image.png" + "W×H" dimensions — not a big thumbnail. The
   chip shows a floating preview of the full image on hover and a remove × on
   hover; the same chip is echoed in the sent user bubble. Images are held per-tab
   as base64 and sent inline as Anthropic image content blocks (built in Rust from
   the {media_type,data} array). WebView2 delivers image blobs on the paste event's
   clipboardData, so we read the clipboard directly in JS — no CLI round-trip. */

/** Per-tab pending images: [{ media_type, data(base64), url(dataURL), w, h, name }]. */
function pendingImages(tab) {
  const t = tab || activeTab();
  if (!t) return [];
  if (!t.images) t.images = [];
  return t.images;
}
/** @returns {boolean} whether the active tab has any pending image (send-enable). */
function hasPendingImages() { const t = activeTab(); return !!(t && t.images && t.images.length); }

/** JSON the Rust side turns into content blocks (data only — no preview fields). An
 *  uploaded file that isn't a pasted image carries its own block already: a document
 *  for one whose contents go inline, a text block naming the path for the rest. */
function pendingImagesJson(tab) {
  return JSON.stringify(pendingImages(tab).map(im =>
    im.block ? im.block : { media_type: im.media_type, data: im.data }));
}

/** Clipboard bitmaps have no filename; VSCode labels them "image.<ext>". */
function imageName(mediaType) {
  const ext = ((mediaType || 'image/png').split('/')[1] || 'png').split('+')[0];
  return 'image.' + ext;
}

/**
 * Adds one image (from a data URL); measures W×H async, then re-renders.
 * @param {string} dataUrl
 * @param {Tab} [tab]  the conversation it belongs to — defaults to the active one. A
 *   download that finishes after the user switched tabs still lands where it was pasted,
 *   and only redraws the strip when that tab is the one on screen (switching back
 *   re-renders anyway).
 * @param {string} [name]  an uploaded file's own name; a paste has none ("image.png").
 */
function addPendingImage(dataUrl, tab, name) {
  const m = /^data:([^;,]+)(?:;base64)?,(.*)$/.exec(dataUrl || '');
  if (!m) return;
  const media_type = m[1] || 'image/png';
  const data = m[2] || '';
  if (!data) return;
  const im = { media_type, data, url: dataUrl, w: 0, h: 0, name: name || imageName(media_type) };
  pendingImages(tab).push(im);
  const onScreen = () => !tab || tab === activeTab();
  // Read natural dimensions off-screen, then refresh the chip's "W×H".
  const probe = new Image();
  probe.onload = () => {
    im.w = probe.naturalWidth; im.h = probe.naturalHeight;
    if (onScreen()) renderPendingImages();
  };
  probe.src = dataUrl;
  if (!onScreen()) return;
  renderPendingImages();
  syncComposer();
}

/**
 * Rebuilds a chip-ready image from a stored transcript block. Session history
 * gives back only {media_type, data} — the data URL, name and (lazily, in
 * makeImageChip) the dimensions are derived here.
 * @param {{media_type?: string, data?: string}} b
 */
function imageFromBlock(b) {
  const mt = (b && b.media_type) || 'image/png';
  const data = (b && b.data) || '';
  if (!data) return null;
  return { media_type: mt, data, url: 'data:' + mt + ';base64,' + data, w: 0, h: 0, name: imageName(mt) };
}

/**
 * Rebuilds an attachment chip from a reloaded transcript's `{title, encoding, index}`.
 * The uploaded file's contents stay in the transcript — clicking the chip asks the host
 * for that one file (see _openStoredAttachment), so reopening a conversation never drags
 * a 30MB upload through the page. A file that went in as a path kept the path instead.
 * @param {string} [uuid]      the transcript id of the message the chip belongs to
 * @param {string} [sessionId] the conversation it was loaded from
 */
function documentFromBlock(d, uuid, sessionId) {
  if (!d) return null;
  if (d.encoding === 'path') {
    return d.path ? pathAttachment(d.title || basename(d.path), d.path) : null;
  }
  if (!uuid || !sessionId) return null;
  return { kind: 'document', name: d.title || 'Document', path: '',
           ref: { uuid, sessionId, index: d.index || 0 } };
}

/* ---- Upload from computer ----
   Sorted the way the extension sorts them: PNG/JPG/GIF/WebP go in as images, PDFs and
   text files as documents. What the extension refuses — an archive, a binary, anything
   else — goes in as its path instead, which is what that refusal tells you to do by
   hand: Claude reads it with its own tools. */
const UPLOAD_IMAGE_TYPES = ['image/jpeg', 'image/png', 'image/gif', 'image/webp'];
const UPLOAD_TEXT_TYPES = ['application/json', 'application/xml', 'application/javascript', 'application/typescript',
  'application/x-javascript', 'application/x-typescript', 'application/x-yaml', 'application/yaml', 'application/x-sh',
  'application/x-shellscript', 'application/sql', 'application/graphql', 'application/toml', 'application/x-toml'];
const UPLOAD_TEXT_EXTS = new Set(['json', 'yaml', 'yml', 'toml', 'ini', 'cfg', 'conf', 'config', 'env', 'properties', 'js',
  'jsx', 'ts', 'tsx', 'mjs', 'cjs', 'mts', 'cts', 'py', 'pyw', 'rb', 'go', 'rs', 'java', 'kt', 'kts', 'scala', 'c', 'h', 'cpp',
  'hpp', 'cc', 'cxx', 'cs', 'fs', 'fsx', 'swift', 'php', 'pl', 'pm', 'lua', 'r', 'jl', 'ex', 'exs', 'erl', 'hrl', 'clj', 'cljs',
  'cljc', 'elm', 'hs', 'ml', 'mli', 'v', 'sv', 'vhd', 'vhdl', 'asm', 's', 'html', 'htm', 'xhtml', 'xml', 'svg', 'css', 'scss',
  'sass', 'less', 'vue', 'svelte', 'astro', 'sh', 'bash', 'zsh', 'fish', 'ps1', 'psm1', 'psd1', 'bat', 'cmd', 'csv', 'tsv',
  'sql', 'graphql', 'gql', 'prisma', 'md', 'mdx', 'markdown', 'rst', 'txt', 'text', 'rtf', 'tex', 'latex', 'org', 'adoc',
  'asciidoc', 'makefile', 'cmake', 'gradle', 'dockerfile', 'containerfile', 'vagrantfile', 'rakefile', 'gemfile', 'podfile',
  'fastfile', 'brewfile', 'procfile', 'lock', 'sum', 'log', 'diff', 'patch', 'gitignore', 'gitattributes', 'editorconfig',
  'prettierrc', 'eslintrc', 'babelrc', 'npmrc', 'nvmrc', 'yarnrc']);

/** @returns {'image'|'pdf'|'text'|'unsupported'} */
function attachmentKind(type, name) {
  if (UPLOAD_IMAGE_TYPES.includes(type)) return 'image';
  if (type === 'application/pdf') return 'pdf';
  if (type.startsWith('text/') || UPLOAD_TEXT_TYPES.includes(type)) return 'text';
  const ext = name.split('.').pop().toLowerCase();
  if (ext && UPLOAD_TEXT_EXTS.has(ext)) return 'text';
  const lower = name.toLowerCase();
  if (UPLOAD_TEXT_EXTS.has(lower) || ['license', 'readme', 'changelog', 'authors', 'contributors', 'copying'].includes(lower)) return 'text';
  return 'unsupported';
}

function basename(p) { return String(p || '').split(/[\\/]/).pop(); }

/** A chip for a file that goes in as its path — the block names the path for the model. */
function pathAttachment(name, path) {
  return { kind: 'path', name, path,
           block: { type: 'text', text: '<attached_file path="' + path + '" />' } };
}

function pickFilesFromComputer() {
  closeMenus();
  if (window._pickFiles) window._pickFiles();
}

window.onFilesPicked = function () {
  if (!window._drainPickedFiles) return;
  let files = [];
  try { files = JSON.parse(window._drainPickedFiles() || '[]'); } catch (e) { return; }
  const tooLarge = [];
  files.forEach(f => {
    const type = String(f.type || '').toLowerCase();
    const name = String(f.name || '');
    // Over the cap, the host sends the size instead of the contents — no chip.
    if (f.tooLarge) { tooLarge.push(name + ' is ' + (Number(f.tooLarge) / 1048576).toFixed(1) + 'MB'); return; }
    switch (attachmentKind(type, name)) {
      case 'image':
        addPendingImage('data:' + type + ';base64,' + f.data, null, name);
        break;
      case 'pdf':
        addPendingDocument(name, f.path, { type: 'document', source: { type: 'base64', media_type: 'application/pdf', data: f.data }, title: name });
        break;
      case 'text': {
        const text = new TextDecoder().decode(Uint8Array.from(atob(f.data || ''), c => c.charCodeAt(0)));
        addPendingDocument(name, f.path, { type: 'document', source: { type: 'text', media_type: 'text/plain', data: text }, title: name });
        break;
      }
      default:
        // Not something the API takes inline: the path goes instead, so Claude can
        // reach for the file with its own tools.
        addPendingAttachment(pathAttachment(name, f.path));
    }
  });
  const t = activeTab();
  if (tooLarge.length && t) addSystemTo(t, '⚠ File too large (max 32MB): ' + tooLarge.join(', ') + '.');
};

function addPendingDocument(name, path, block) {
  if (!block.source.data) return;
  addPendingAttachment({ kind: 'document', name, path: path || '', block });
}

/** Adds a non-image chip (a document or a path) to the active tab's strip. */
function addPendingAttachment(im) {
  pendingImages().push(im);
  renderPendingImages();
  syncComposer();
}

/** Opens a chip's file the way the OS opens that kind of file. A chip from a reloaded
 *  conversation names its place in the transcript and the host writes that file out; a
 *  pending one carries the path it was picked from and its contents. */
function openAttachment(im) {
  if (im.ref) {
    if (window._openStoredAttachment) {
      window._openStoredAttachment(rootPathOf(activeTab()) || '', im.ref.sessionId, im.ref.uuid, im.ref.index);
    }
    return;
  }
  const src = (im.block && im.block.source) || {};
  if (window._openAttachment) {
    window._openAttachment(im.path || '', im.name || '', src.data || '', src.type === 'base64');
  }
}

/** Reads a pasted image File/Blob into a data URL, then adds it. */
function readPastedImage(file) {
  if (!file) return;
  const r = new FileReader();
  r.onload = () => addPendingImage(String(r.result || ''));
  r.readAsDataURL(file);
}

function clearPendingImages(tab) {
  const t = tab || activeTab();
  if (t) t.images = [];
  renderPendingImages();
}

/**
 * Builds a VSCode-style image chip: tiny thumbnail + name + dimensions. CLICKING
 * the chip body opens the full image in a centered lightbox (joebiden5); hovering
 * only shows the native filename tooltip. Removable chips also get an × on hover.
 * @param {{url: string, name: string, w: number, h: number}} im
 * @param {(() => void)|null} onRemove  composer chips pass a remover; bubble chips don't
 */
function makeImageChip(im, onRemove) {
  const chip = document.createElement('span'); chip.className = 'img-chip';
  // Everything except the × is one click target that opens the preview.
  const open = document.createElement('span'); open.className = 'ic-open';
  open.title = im.name || 'image.png';
  if (im.kind === 'document' || im.kind === 'path') {
    // An uploaded non-image file: the file icon and its own name, opening the file itself.
    const icon = document.createElement('span'); icon.className = 'ic-file'; icon.innerHTML = ICONS.FILEICON;
    const label = document.createElement('span'); label.className = 'ic-name'; label.textContent = im.name;
    open.appendChild(icon); open.appendChild(label);
    open.onclick = () => openAttachment(im);
    chip.appendChild(open);
    return withRemove(chip, onRemove);
  }
  const thumb = document.createElement('span'); thumb.className = 'ic-thumb';
  thumb.style.backgroundImage = 'url("' + im.url + '")';
  const name = document.createElement('span'); name.className = 'ic-name'; name.textContent = im.name || 'image.png';
  const dim = document.createElement('span'); dim.className = 'ic-dim';
  if (im.w && im.h) dim.textContent = im.w + '×' + im.h;
  else {
    // A chip rebuilt from history has no measured size yet — read it off-screen
    // and fill this chip's label in place (no re-render; the chip may live in a
    // reloaded bubble rather than the composer strip).
    const probe = new Image();
    probe.onload = () => {
      im.w = probe.naturalWidth; im.h = probe.naturalHeight;
      dim.textContent = im.w + '×' + im.h;
    };
    probe.src = im.url;
  }
  open.appendChild(thumb); open.appendChild(name); open.appendChild(dim);
  open.onclick = () => openLightbox(im);
  chip.appendChild(open);
  return withRemove(chip, onRemove);
}

/** Gives a composer chip its hover × (sent bubbles pass no remover). */
function withRemove(chip, onRemove) {
  if (onRemove) {
    // Marks the chip as the kind that reveals an × on hover, so a sent bubble's chips
    // — which have no × — don't dim their dimensions for nothing.
    chip.classList.add('removable');
    const x = document.createElement('span'); x.className = 'ic-x'; x.innerHTML = ICONS.X; x.title = 'Remove';
    // Removing must not also open the preview.
    x.onclick = (e) => { e.stopPropagation(); onRemove(); };
    chip.appendChild(x);
  }
  return chip;
}

/* ---- lightbox ----
   Centered full image over a dimmed backdrop. Dismiss: × badge, backdrop click,
   or Esc. The Esc listener is capture-phase and registered only while open, so
   it takes precedence over the composer/menu Esc handlers, matching the other
   dialogs here (rewind, cards). */
function lightboxKey(e) {
  if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); closeLightbox(); }
}
function openLightbox(im) {
  const box = document.getElementById('lightbox');
  if (!box || !im) return;
  const img = box.querySelector('img');
  img.src = im.url;
  img.alt = im.name || 'image';
  box.classList.add('open');
  // Tooltip names the key Eclipse actually has bound (Esc, or Ctrl+G under Emacs, where Esc
  // is a multi-stroke prefix the page never receives). The markup names no key at all, so
  // the worst case here is the bare "Close preview" rather than a wrong one.
  registerHintPainter(box.querySelector('.lb-x'), function paintCloseTitle() {
    const x = box.querySelector('.lb-x');
    if (!x) return;
    const k = cancelKeyName();
    x.title = k ? 'Close preview (' + k + ')' : 'Close preview';
  });
  document.addEventListener('keydown', lightboxKey, true);
  registerOverlayCancel(closeLightbox, false);   // not tab-owned — no visibility guard
}
function closeLightbox() {
  const box = document.getElementById('lightbox');
  if (!box) return;
  box.classList.remove('open');
  const img = box.querySelector('img');
  if (img) img.src = '';        // release the data URL
  document.removeEventListener('keydown', lightboxKey, true);
  unregisterOverlayCancel();
}
/* Backdrop click closes; clicks on the image itself don't. */
(function wireLightbox() {
  const box = document.getElementById('lightbox');
  if (!box) return;
  box.onclick = (e) => { if (e.target === box) closeLightbox(); };
  const x = box.querySelector('.lb-x');
  if (x) x.onclick = (e) => { e.stopPropagation(); closeLightbox(); };
})();

/** Renders the active tab's pending-image chips into the composer strip. */
function renderPendingImages() {
  const strip = document.getElementById('pending-images');
  if (!strip) return;
  const imgs = pendingImages();
  // The strip is one horizontally scrolled row, and it's rebuilt on every change —
  // hold its scroll position so removing a chip doesn't fling the row back to the start.
  const scrollLeft = strip.scrollLeft;
  strip.innerHTML = '';
  strip.classList.toggle('show', imgs.length > 0);
  imgs.forEach((im, i) => {
    strip.appendChild(makeImageChip(im, () => {
      imgs.splice(i, 1); renderPendingImages(); syncComposer();
    }));
  });
  strip.scrollLeft = scrollLeft;
}

/* Capture image paste on the composer. A screenshot paste arrives as a file item
   with an image/* MIME type; when present we consume the event so the textarea
   doesn't also paste a filename/text alternative. Plain-text paste is untouched.

   Only Ctrl+V produces this DOM event at all — see the comment on window.__ccLastPaste
   in contextmenu.js for why ccPaste() (the host's org.eclipse.ui.edit.paste handler,
   and the right-click menu's Paste item) needs to know this one just happened. */
input.addEventListener('paste', (e) => {
  window.__ccLastPaste = Date.now();
  const data = e.clipboardData;
  if (!data) return;
  let took = false;
  for (const item of data.items || []) {
    if (item.kind === 'file' && item.type && item.type.indexOf('image/') === 0) {
      const file = item.getAsFile();
      if (file) { readPastedImage(file); took = true; }
    }
  }
  if (took) e.preventDefault();
});
