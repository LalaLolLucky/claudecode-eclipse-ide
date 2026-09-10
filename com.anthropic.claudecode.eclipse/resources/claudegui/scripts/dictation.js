/* dictation.js — mic button, hold/tap gesture, and the interim→final transcript
   render. Audio never touches this layer: capture and transcription both live in
   the Rust core (see stt.rs), so there is no getUserMedia call here and no
   WebView2 microphone permission to grant. This file only starts/stops the
   native side and paints what comes back. */

const micField = document.getElementById('input-field');
const micMirror = document.getElementById('input-mirror');
const micBtn = document.getElementById('mic-btn');
const micDock = document.getElementById('mic-dock');
const micLevel = document.getElementById('mic-level');
const micBars = micLevel ? Array.from(micLevel.querySelectorAll('i')) : [];

/* Measured off the reference: 6px at rest, ~16px at peak, and the middle bar
   runs tallest with the outer two trailing it. */
const MIC_BAR_MIN = 6;
const MIC_BAR_MAX = 16;
const MIC_BAR_BIAS = [0.75, 1.0, 0.85];

let micRecording = false;
/* The composer split around the caret at the moment dictation began, plus the
   finalised segments so far. Speech is rendered as prefix + spoken + suffix, so
   it lands AT THE CURSOR rather than at the end -- put the caret mid-word and
   the transcript is inserted there, leaving the tail intact.

   micDetached latches once the user starts typing during a take: from then on
   nothing further is written to the composer, because the anchors no longer
   describe where their text is and a late transcript would clobber it. */
let micPrefix = '';
let micSuffix = '';
let micFinal = '';
let micDetached = false;

/* Renders `spoken` between the two anchors and leaves the caret at its end, so
   typing straight after dictation continues from the right place.

   Programmatic writes don't fire 'input', which is what autosizes the textarea,
   toggles the send button and drives the slash menu (ui.js) -- so it is
   dispatched by hand. Deliberately NOT 'beforeinput': that one is the signal
   that the USER is editing, and firing it here would look like typing and
   abandon the take. */
/* Text -> markup. MUST escape: both the transcript and whatever the user typed
   around it end up in innerHTML, and neither is trusted input. */
function micEsc(t) {
  return String(t).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

/* Redraws the decorative layer: the same text as the textarea, with only the
   current take's words italic. Called from micRender, so the two can never show
   different text. */
function micPaintMirror(spoken, tail) {
  /* Guarded on the class rather than trusting call order: onSttDone drops the
     mirror and THEN renders the final text, so an unguarded painter would put
     the italics straight back after the take had ended. */
  if (!micMirror || !micField || !micField.classList.contains('mirroring')) return;
  const full = micPrefix + spoken + tail;
  let html = micEsc(micPrefix)
           + (spoken ? '<i>' + micEsc(spoken) + '</i>' : '')
           + micEsc(tail);
  /* A textarea shows a trailing newline as an empty last line; pre-wrap in a div
     collapses it. The zero-width space keeps the two the same height. */
  if (full.endsWith('\n')) html += '&#8203;';
  micMirror.innerHTML = html;
  micMirror.scrollTop = input.scrollTop;
}

function micMirrorOn() {
  if (!micField) return;
  micField.classList.add('mirroring');
}

/* Ends the italic session: the layer goes away and the textarea's own glyphs
   come back, so previously dictated text reads as ordinary text from here on. */
function micMirrorOff() {
  if (!micField) return;
  micField.classList.remove('mirroring');
  if (micMirror) micMirror.innerHTML = '';
}

/* The field scrolls once it passes its max height; the copy has to follow. */
input.addEventListener('scroll', () => {
  if (micMirror && micField && micField.classList.contains('mirroring')) {
    micMirror.scrollTop = input.scrollTop;
  }
});

function micRender(spoken) {
  if (micDetached) return;
  let tail = micSuffix;
  if (tail && spoken && !/^\s/.test(tail)) tail = ' ' + tail;
  input.value = micPrefix + spoken + tail;
  const caret = micPrefix.length + spoken.length;
  try { input.setSelectionRange(caret, caret); } catch (e) { /* detached node */ }
  input.dispatchEvent(new Event('input'));
  input.scrollTop = input.scrollHeight;
  micPaintMirror(spoken, tail);
}

function micStart() {
  if (micRecording || micBtn.classList.contains('busy')) return;
  const value = input.value;
  const from = typeof input.selectionStart === 'number' ? input.selectionStart : value.length;
  const to   = typeof input.selectionEnd   === 'number' ? input.selectionEnd   : value.length;
  micPrefix = value.slice(0, from);
  micSuffix = value.slice(to);          // a selection is replaced, as typing would
  if (micPrefix && !/\s$/.test(micPrefix)) micPrefix += ' ';
  micFinal = '';
  micDetached = false;
  micMirrorOn();
  micPaintMirror('', micSuffix);
  micRecording = true;
  micBtn.classList.add('recording');
  micDock.classList.add('recording');
  micLevel.hidden = false;
  micSetLevel(0);
  try { window._sttStart && window._sttStart(); } catch (e) { micFail('' + e); }
}

function micStop() {
  if (!micRecording) return;
  micRecording = false;
  micBtn.classList.remove('recording');
  micDock.classList.remove('recording');
  micLevel.hidden = true;
  /* Held until onSttDone: the tail of the audio is still being transcribed, and
     a second start before it lands would interleave two transcripts. */
  micBtn.classList.add('busy');
  try { window._sttStop && window._sttStop(); } catch (e) { micFail('' + e); }
}

/** Keyboard/command entry point: always a latching toggle, never a hold. */
function micToggle() {
  if (micRecording) { micLatched = false; micStop(); }
  else { micLatched = true; micStart(); }
}

function micFail(msg) {
  micRecording = false;
  micLatched = false;
  micPressActive = false;
  micBtn.classList.remove('recording');
  micBtn.classList.remove('busy');
  micDock.classList.remove('recording');
  micLevel.hidden = true;
  micMirrorOff();
  if (typeof addSystem === 'function') addSystem('⚠ Dictation: ' + msg);
}

/* ---- gesture ----
   Both behaviours, as VS Code does them:

     press and hold  -> records while held, stops the moment the button is released
     quick tap       -> latches on, and stays on until the button is tapped again

   Which one it was is decided on RELEASE, from how long the button was down.

   The release is caught on the DOCUMENT, not on the button. An earlier version
   used `mouseleave`, which fires on ordinary pointer movement -- so simply
   moving the mouse away ended the take. Listening on the document means a press
   that drags off the button and releases somewhere else still ends correctly,
   while moving the pointer around with no button down changes nothing. */
const MIC_HOLD_MS = 400;

let micPressAt = 0;
let micPressActive = false;
let micPressFromLatched = false;   // the press began while already latched on
let micLatched = false;

micBtn.addEventListener('mousedown', (e) => {
  if (e.button !== 0) return;      // left button only
  e.preventDefault();              // no text selection while holding
  micPressActive = true;
  micPressAt = Date.now();
  micPressFromLatched = micLatched;
  if (!micRecording) micStart();
});

document.addEventListener('mouseup', () => {
  if (!micPressActive) return;
  micPressActive = false;
  const heldLongEnough = Date.now() - micPressAt >= MIC_HOLD_MS;

  if (micPressFromLatched) {       // a click while latched = stop
    micLatched = false;
    micStop();
    return;
  }
  if (heldLongEnough) {            // it was a hold; release ends it
    micLatched = false;
    micStop();
  } else {                         // it was a tap; stay on until tapped again
    micLatched = true;
  }
});

/* Paints the meter. `rms` is linear and speech sits low in that range, so it is
   curved before use -- a raw linear map leaves the bars visually dead. */
function micSetLevel(rms) {
  const v = Math.max(0, Math.min(1, Math.sqrt(rms) * 2.2));
  for (let i = 0; i < micBars.length; i++) {
    const h = MIC_BAR_MIN + (MIC_BAR_MAX - MIC_BAR_MIN) * v * (MIC_BAR_BIAS[i] || 1);
    micBars[i].style.height = h.toFixed(1) + 'px';
  }
}

/* Typing takes over: any real edit to the composer during a take ends it and
   hands the field back to the user.

   'beforeinput' rather than 'input' because it fires ONLY for user editing --
   typing, paste, cut, delete. Setting .value from micRender does not raise it,
   so the transcript writing itself can never be mistaken for typing. It also
   fires BEFORE the character lands, so the take is already ending by the time
   the composer changes.

   micDetached is set first: stopping is asynchronous, and the tail transcript
   would otherwise arrive a moment later and overwrite whatever was just typed. */
input.addEventListener('beforeinput', () => {
  if (!micRecording) return;
  micDetached = true;
  micLatched = false;
  micPressActive = false;
  micMirrorOff();          // their text is theirs; nothing stays italic
  micStop();
});

/* No keydown handler here on purpose. Ctrl+D is a real Eclipse command
   (commands.toggleDictation), scoped to the composer context and rebindable
   under Preferences > General > Keys; the handler calls window.micToggle().
   Duplicating it in JS would keep firing the old key after a user rebinds it,
   and would double-fire whenever Eclipse did NOT consume the keystroke. */

/* ---- native callbacks (Java → JS via browser.execute) ---- */

/** Input loudness while recording, ~10x a second. */
window.onSttLevel = (rms) => {
  if (!micRecording) return;
  micSetLevel(parseFloat(rms) || 0);
};

/** Joins two transcript spans without doubling or dropping the gap between them. */
function micJoin(a, b) {
  if (!a) return b;
  if (!b) return a;
  return /\s$/.test(a) ? a + b : a + ' ' + b;
}

/** Revised guess at the current utterance; replaced wholesale by the next one. */
window.onSttPartial = (text) => {
  if (!micRecording) return;
  micRender(micJoin(micFinal, text || ''));
};

/** A settled segment. Appended; later partials build on top of it. */
window.onSttFinal = (text) => {
  if (!(text || '').trim()) return;
  micFinal = micJoin(micFinal, text.trim());
  micRender(micFinal);
};

/** Tail flushed — the transcript is settled and the composer is plain text again. */
window.onSttDone = () => {
  micBtn.classList.remove('busy');
  micMirrorOff();
  micRender(micFinal);
  input.focus();
};

window.onSttError = (msg) => micFail(msg || 'unavailable');
