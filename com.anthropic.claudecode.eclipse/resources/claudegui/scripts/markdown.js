/* markdown.js — Markdown renderer.
   CommonMark blocks and inlines (setext headings, lists with several paragraphs, indented and fenced
   code, reference links, hard breaks, escapes), the GitHub extensions (tables with alignment, task
   lists, strikethrough, bare URLs, footnotes, alerts) and the common extras of other flavors
   (definition lists, abbreviations, heading ids and attribute lists, containers, front matter, a
   table of contents, ==mark==, ++ins++, sub/superscript, [[wiki links]], @mentions, #tags, LaTeX
   math, mermaid flowcharts). Everything is escaped first and only a short list of tags is let
   through again, so what comes out is safe to put into innerHTML. */

/* ===================== Markdown rendering ===================== */
function escapeHtml(s) {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}
function mdEscRe(s) { return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'); }

/* What the render in progress collects: placeholders for finished HTML, link and footnote
   definitions, abbreviations, headings (for ids and the table of contents). */
let MD = null;
function mdNewCtx() {
  return { stash: [], refs: {}, footnotes: {}, fnOrder: [], abbrs: {}, headings: [], ids: {}, toc: false };
}
function mdStash(html) { MD.stash.push(html); return '\u0000' + (MD.stash.length - 1) + '\u0000'; }
function mdUnstash(s) {
  for (let n = 0; n < 4 && s.indexOf('\u0000') >= 0; n++) {
    s = s.replace(/\u0000(\d+)\u0000/g, (_, i) => MD.stash[+i] === undefined ? '' : MD.stash[+i]);
  }
  return s;
}
/* Links and images only to places that cannot run script. `u` is already escaped. A finished
   piece held back (\u0000n\u0000) is never part of one: put back inside the attribute, its own
   quotes would end it, and what follows would be read as attributes of the tag. */
function mdSafeUrl(u) {
  if (u.indexOf('\u0000') >= 0) return '#';
  const raw = u.replace(/&amp;/g, '&').replace(/[\u0000- ]/g, '');
  const m = /^([a-z][a-z0-9+.\-]*):/i.exec(raw);
  if (!m) return u;
  const scheme = m[1].toLowerCase();
  if (scheme === 'http' || scheme === 'https' || scheme === 'mailto' || scheme === 'file') return u;
  if (scheme === 'data' && /^data:image\//i.test(raw)) return u;
  return '#';
}

/* ---------- inline ---------- */
const MD_INLINE_TAGS = 'sub|sup|kbd|mark|ins|del|b|i|u|s|em|strong|small|br';
/* Python's own decorators, which an @mention would otherwise take. */
const MD_DECORATORS = /^(property|staticmethod|classmethod|dataclass|abstractmethod|cached_property|contextmanager|override|overload|wraps|lru_cache|cache)$/;
/* The Java annotations prose names most (the language's, JUnit's, Spring's, Lombok's), which an
   @mention would otherwise take. A handle with a capital, @LalaLolLucky, is still a mention. */
const MD_ANNOTATIONS = /^(Override|Deprecated|SuppressWarnings|FunctionalInterface|SafeVarargs|Retention|Target|Documented|Inherited|Test|ParameterizedTest|RepeatedTest|TestFactory|BeforeEach|AfterEach|BeforeAll|AfterAll|Before|After|BeforeClass|AfterClass|Ignore|Disabled|DisplayName|Nested|Mock|Spy|InjectMocks|Captor|Autowired|Inject|Component|Service|Repository|Controller|RestController|Configuration|Bean|Transactional|Entity|Nullable|NonNull|Nonnull|NotNull|Getter|Setter|Builder|Slf4j|JsonProperty|JsonIgnore)$/;
/* Preprocessor and C# directives, which a #tag would otherwise take. */
const MD_DIRECTIVES = /^(include|define|undef|if|ifdef|ifndef|elif|elifdef|elifndef|else|endif|pragma|error|warning|line|import|region|endregion|nullable|using)$/;

/* A pair of markers counts only when it does not sit inside a word: "10~20 and 30~40", "a==b and c==d"
   and "C++/Java and C++/Rust" are not formatting. */
function mdBounded(match, at, whole) {
  return !/\w/.test(whole[at - 1] || '') && !/\w/.test(whole[at + match.length] || '');
}
/* Inline markdown. Input is raw (unescaped) markdown; output is safe HTML. */
function mdInline(s) {
  const own = !MD;
  if (own) MD = mdNewCtx();
  try { return mdInlineRun(s); } finally { if (own) MD = null; }
}
function mdInlineRun(s) {
  s = s.replace(/\u0000/g, '');
  s = s.replace(/\\\n/g, () => mdStash('<br>'));
  s = s.replace(/\\([\\`*_{}\[\]()#+\-.!|~=^<>$@:"'&])/g, (_, c) => mdStash(escapeHtml(c)));
  // code spans first, so nothing formats their contents
  s = s.replace(/(`+)(?!`)([\s\S]*?[^`])\1(?!`)/g, (_, t, c) => {
    if (c.length > 2 && c[0] === ' ' && c[c.length - 1] === ' ' && /\S/.test(c)) c = c.slice(1, -1);
    return mdStash('<code class="ic">' + escapeHtml(c) + '</code>');
  });
  // inline math: $x$, but not "$5 and $6", nor shell variables run together ("$PATH:$HOME",
  // "${A}/${B}"): a closing $ is not followed by a digit, a letter, a brace or another $
  s = s.replace(/\$([^\s$](?:[^$\n]*?[^\s$\\])?)\$(?![\w${])/g, (_, t) => mdStash(mdMath(t, false)));
  s = escapeHtml(s);

  // <https://…> and <me@example.com>. What goes into an attribute (a URL, an alt, a title) may
  // not take in a finished piece (\u0000n\u0000): see mdSafeUrl.
  s = s.replace(/&lt;((?:https?:\/\/|mailto:)[^\s\u0000]*?)&gt;/g, (_, u) => mdStash('<a href="' + mdSafeUrl(u) + '">' + u + '</a>'));
  s = s.replace(/&lt;([^\s@&\u0000]+@[^\s@&\u0000]+\.[A-Za-z]+)&gt;/g, (_, u) => mdStash('<a href="mailto:' + u + '">' + u + '</a>'));
  // the few HTML tags that are allowed through
  s = s.replace(new RegExp('&lt;(\\/?)(' + MD_INLINE_TAGS + ')\\s*(\\/?)&gt;', 'gi'),
    (_, c, t) => mdStash('<' + c + t.toLowerCase() + '>'));
  s = s.replace(/&lt;abbr\s+title=&quot;([^&"\u0000]*)&quot;\s*&gt;/gi, (_, t) => mdStash('<abbr title="' + t + '">'));
  s = s.replace(/&lt;\/abbr&gt;/gi, () => mdStash('</abbr>'));

  // images and links ([text](url "title"))
  s = s.replace(/!\[([^\]\u0000]*)\]\(([^)\s\u0000]+)(?:\s+&quot;([^\u0000]*?)&quot;)?\)/g, (_, alt, src, title) =>
    mdStash('<img alt="' + alt + '" src="' + mdSafeUrl(src) + '"' + (title ? ' title="' + title + '"' : '') + '>'));
  s = s.replace(/\[((?:[^\[\]]|\[[^\]]*\])+)\]\(([^)\s\u0000]+)(?:\s+&quot;([^\u0000]*?)&quot;)?\)/g, (m, text, url, title) => {
    const open = '<a href="' + mdSafeUrl(url) + '"' + (title ? ' title="' + title + '"' : '') + '>';
    return text.indexOf('://') >= 0 ? mdStash(open + text + '</a>') : mdStash(open) + text + mdStash('</a>');
  });
  // footnote references, wiki links, reference-style links
  s = s.replace(/\[\^([^\]\s]+)\]/g, (m, id) => {
    if (!(id in MD.footnotes)) return m;
    let n = MD.fnOrder.indexOf(id);
    const first = n < 0;
    if (first) { MD.fnOrder.push(id); n = MD.fnOrder.length - 1; }
    const fid = id.replace(/[^\w-]/g, '_');
    return mdStash('<sup class="fn-ref"><a ' + (first ? 'id="fnref-' + fid + '" ' : '') + 'href="#fn-' + fid + '">' + (n + 1) + '</a></sup>');
  });
  s = s.replace(/\[\[([^\]\n]+)\]\]/g, (_, t) => mdStash('<span class="md-wiki">' + t + '</span>'));
  s = s.replace(/\[([^\]]+)\]\[([^\]]*)\]/g, (m, text, id) => {
    const r = MD.refs[(id || text).toLowerCase()];
    if (!r) return m;
    return mdStash('<a href="' + mdSafeUrl(r.url) + '"' + (r.title ? ' title="' + r.title + '"' : '') + '>') + text + mdStash('</a>');
  });
  s = s.replace(/\[([^\[\]^][^\[\]]*)\](?!\()/g, (m, text) => {
    const r = MD.refs[text.toLowerCase()];
    if (!r) return m;
    return mdStash('<a href="' + mdSafeUrl(r.url) + '"' + (r.title ? ' title="' + r.title + '"' : '') + '>') + text + mdStash('</a>');
  });

  // emphasis and the extras
  s = s.replace(/\*\*\*(?=\S)([\s\S]*?\S)\*\*\*/g, '<strong><em>$1</em></strong>');
  s = s.replace(/\*\*(?=\S)([\s\S]*?\S)\*\*/g, '<strong>$1</strong>');
  s = s.replace(/(^|[^\w])__(?=\S)([\s\S]*?\S)__(?!\w)/g, '$1<strong>$2</strong>');
  s = s.replace(/\*([^\s*](?:[^*\n]*?[^\s*])?)\*/g, '<em>$1</em>');
  s = s.replace(/(^|[^\w])_([^\s_](?:[^_\n]*?[^\s_])?)_(?!\w)/g, '$1<em>$2</em>');
  s = s.replace(/~~(?=\S)([\s\S]*?\S)~~/g, '<del>$1</del>');
  // one tilde: H~2~O is a subscript, ~crossed out words~ (with spaces) a strikethrough
  s = s.replace(/~([^\s~](?:[^~\n]*?[^\s~])?)~/g, (m, t, at, whole) => {
    if (!/\s/.test(t)) return '<sub>' + t + '</sub>';
    return mdBounded(m, at, whole) ? '<del>' + t + '</del>' : m;
  });
  s = s.replace(/\^([^\s^]+)\^/g, '<sup>$1</sup>');
  s = s.replace(/==(?=\S)([\s\S]*?\S)==/g, (m, t, at, whole) => mdBounded(m, at, whole) ? '<mark>' + t + '</mark>' : m);
  s = s.replace(/\+\+(?=\S)([\s\S]*?\S)\+\+/g, (m, t, at, whole) => mdBounded(m, at, whole) ? '<ins>' + t + '</ins>' : m);

  // bare URLs
  s = s.replace(/(^|[\s(>])((?:https?:\/\/|www\.)[^\s<\u0000]*[^\s<.,;:!?)\]'"\u0000])/g, (_, p, u) =>
    p + mdStash('<a href="' + (/^www\./i.test(u) ? 'https://' : '') + mdSafeUrl(u) + '">' + u + '</a>'));
  // @mentions, #123 issues, #tags. Code talked about in prose is not one: a Java annotation
  // (@Override, @Test) or a decorator (@property, @app.route(...)), and a preprocessor or C#
  // directive (#include, #region).
  s = s.replace(/(^|[\s(])@([A-Za-z0-9][A-Za-z0-9_-]*)(?![\w-]|\(|\.\w)/g, (m, p, n) =>
    MD_ANNOTATIONS.test(n) || MD_DECORATORS.test(n) ? m : p + mdStash('<span class="md-mention">@' + n + '</span>'));
  s = s.replace(/(^|[\s(])#(\d+)\b/g, (_, p, n) => p + mdStash('<span class="md-issue">#' + n + '</span>'));
  s = s.replace(/(^|[\s(])#([A-Za-z][\w-]*)/g, (m, p, n) =>
    MD_DIRECTIVES.test(n) ? m : p + mdStash('<span class="md-tag">#' + n + '</span>'));

  // abbreviations, in the text only (not inside a tag or a finished piece)
  const abbrs = Object.keys(MD.abbrs);
  if (abbrs.length) {
    abbrs.sort((a, b) => b.length - a.length);
    const re = new RegExp('\\b(' + abbrs.map(mdEscRe).join('|') + ')\\b', 'g');
    s = s.split(/(<[^>]*>|\u0000\d+\u0000)/).map((part, k) => k % 2 ? part
      : part.replace(re, (m) => '<abbr title="' + MD.abbrs[m] + '">' + m + '</abbr>')).join('');
  }
  return mdUnstash(s);
}

/* ---------- LaTeX ---------- */
const TEX_SYM = {
  alpha: 'α', beta: 'β', gamma: 'γ', delta: 'δ', epsilon: 'ε', varepsilon: 'ε', zeta: 'ζ', eta: 'η', theta: 'θ',
  vartheta: 'ϑ', iota: 'ι', kappa: 'κ', lambda: 'λ', mu: 'μ', nu: 'ν', xi: 'ξ', pi: 'π', rho: 'ρ', sigma: 'σ',
  tau: 'τ', upsilon: 'υ', phi: 'φ', varphi: 'φ', chi: 'χ', psi: 'ψ', omega: 'ω', Gamma: 'Γ', Delta: 'Δ',
  Theta: 'Θ', Lambda: 'Λ', Xi: 'Ξ', Pi: 'Π', Sigma: 'Σ', Phi: 'Φ', Psi: 'Ψ', Omega: 'Ω',
  sum: '∑', prod: '∏', int: '∫', iint: '∬', oint: '∮', infty: '∞', partial: '∂', nabla: '∇', pm: '±', mp: '∓',
  times: '×', cdot: '·', div: '÷', le: '≤', leq: '≤', ge: '≥', geq: '≥', ne: '≠', neq: '≠', approx: '≈',
  equiv: '≡', sim: '∼', simeq: '≃', propto: '∝', to: '→', rightarrow: '→', leftarrow: '←', Rightarrow: '⇒',
  Leftarrow: '⇐', leftrightarrow: '↔', Leftrightarrow: '⇔', mapsto: '↦', in: '∈', notin: '∉', ni: '∋',
  subset: '⊂', subseteq: '⊆', supset: '⊃', supseteq: '⊇', cup: '∪', cap: '∩', emptyset: '∅', varnothing: '∅',
  forall: '∀', exists: '∃', neg: '¬', land: '∧', lor: '∨', wedge: '∧', vee: '∨', ldots: '…', cdots: '⋯',
  dots: '…', vdots: '⋮', angle: '∠', degree: '°', prime: '′', langle: '⟨', rangle: '⟩', lbrace: '{',
  rbrace: '}', lVert: '‖', rVert: '‖', Vert: '‖', vert: '|', ll: '≪', gg: '≫', perp: '⊥', parallel: '∥',
  star: '⋆', circ: '∘', bullet: '•', oplus: '⊕', otimes: '⊗', hbar: 'ℏ', ell: 'ℓ', Re: 'ℜ', Im: 'ℑ',
  aleph: 'ℵ', quad: ' ', qquad: '  ', ',': ' ', ';': ' ', ':': ' ',
  '!': '', ' ': ' '
};
const TEX_FUNCS = ['sin', 'cos', 'tan', 'cot', 'sec', 'csc', 'arcsin', 'arccos', 'arctan', 'sinh', 'cosh', 'tanh',
  'log', 'ln', 'lg', 'exp', 'lim', 'limsup', 'liminf', 'max', 'min', 'sup', 'inf', 'det', 'gcd', 'arg', 'deg',
  'dim', 'ker', 'mod', 'Pr'];

/* Plain-HTML typesetting of the usual LaTeX: no layout engine, so fractions are stacked and roots
   are overlined, scripts are raised or lowered, symbols are the Unicode ones. */
function mdTex(src) {
  let i = 0;
  function skipSpaces() { while (i < src.length && /\s/.test(src[i])) i++; }
  function rawGroup() {          // {…} as plain text
    skipSpaces();
    if (src[i] !== '{') return i < src.length ? src[i++] : '';
    let depth = 0, out = '';
    for (; i < src.length; i++) {
      const c = src[i];
      if (c === '{') { if (depth++ > 0) out += c; }
      else if (c === '}') { if (--depth === 0) { i++; break; } out += c; }
      else out += c;
    }
    return out;
  }
  function atom() {
    skipSpaces();
    if (i >= src.length) return '';
    const c = src[i];
    if (c === '{') { i++; const h = seq(true); if (src[i] === '}') i++; return h; }
    if (c === '\\') {
      i++;
      if (i >= src.length) return '';
      let name = '';
      if (/[A-Za-z]/.test(src[i])) { while (i < src.length && /[A-Za-z]/.test(src[i])) name += src[i++]; }
      else name = src[i++];
      return command(name);
    }
    i++;
    if (/[A-Za-z]/.test(c)) return '<i>' + c + '</i>';
    if (c === '-') return '−';
    if (c === '\'') return '′';
    return escapeHtml(c);
  }
  function command(name) {
    if (name === 'frac' || name === 'dfrac' || name === 'tfrac') {
      const a = atom(), b = atom();
      return '<span class="mfrac"><span class="mnum">' + a + '</span><span class="mden">' + b + '</span></span>';
    }
    if (name === 'sqrt') {
      skipSpaces();
      let n = '';
      if (src[i] === '[') { const j = src.indexOf(']', i); if (j > 0) { n = src.slice(i + 1, j); i = j + 1; } }
      return (n ? '<sup class="mroot">' + escapeHtml(n) + '</sup>' : '') + '<span class="msqrt">√<span class="mrad">' + atom() + '</span></span>';
    }
    if (name === 'text' || name === 'mathrm' || name === 'operatorname' || name === 'textrm' || name === 'mathbf' || name === 'textbf' || name === 'mathit' || name === 'textit' || name === 'mathbb' || name === 'mathcal') {
      const t = escapeHtml(rawGroup());
      if (name === 'mathbf' || name === 'textbf') return '<b>' + t + '</b>';
      if (name === 'mathit' || name === 'textit') return '<i>' + t + '</i>';
      return '<span class="mrm">' + t + '</span>';
    }
    if (name === 'left' || name === 'right') {
      skipSpaces();
      if (src[i] === '\\') { i++; const d = src[i++] || ''; return d === '{' || d === '}' ? d : (TEX_SYM[d] !== undefined ? TEX_SYM[d] : escapeHtml(d)); }
      const d = src[i++] || '';
      return d === '.' ? '' : escapeHtml(d);
    }
    if (name === 'overline' || name === 'bar') return '<span class="mover">' + atom() + '</span>';
    if (name === 'underline') return '<span class="mund">' + atom() + '</span>';
    if (name === 'hat' || name === 'vec' || name === 'tilde' || name === 'dot') {
      const mark = { hat: '^', vec: '→', tilde: '~', dot: '·' }[name];
      return '<span class="macc"><span class="macm">' + mark + '</span>' + atom() + '</span>';
    }
    if (name === '\\') return '<br>';
    if (name === '{' || name === '}' || name === '%' || name === '$' || name === '&' || name === '#' || name === '_') return escapeHtml(name);
    if (TEX_FUNCS.indexOf(name) >= 0) return '<span class="mrm">' + name + '</span>';
    if (name in TEX_SYM) return TEX_SYM[name];
    return '<span class="mrm">' + escapeHtml(name) + '</span>';
  }
  function seq(inBraces) {
    let out = '';
    while (i < src.length) {
      skipSpaces();
      if (i >= src.length) break;
      if (inBraces && src[i] === '}') break;
      if (src[i] === '^' || src[i] === '_') {   // a script with nothing in front of it
        const up = src[i] === '^'; i++;
        out += (up ? '<sup>' : '<sub>') + atom() + (up ? '</sup>' : '</sub>');
        continue;
      }
      const a = atom();
      let sup = '', sub = '';
      for (;;) {
        skipSpaces();
        if (src[i] === '^' && !sup) { i++; sup = atom(); }
        else if (src[i] === '_' && !sub) { i++; sub = atom(); }
        else break;
      }
      if (sup && sub) out += a + '<span class="ms"><sup>' + sup + '</sup><sub>' + sub + '</sub></span>';
      else out += a + (sup ? '<sup>' + sup + '</sup>' : '') + (sub ? '<sub>' + sub + '</sub>' : '');
    }
    return out;
  }
  return seq(false);
}
function mdMath(tex, display) {
  const html = mdTex(tex.trim());
  return display ? '<div class="md-math md-math-block">' + html + '</div>' : '<span class="md-math">' + html + '</span>';
}

/* ---------- mermaid flowcharts ---------- */
let MD_DIAGRAMS = 0;
/* graph/flowchart only; any other kind of diagram stays a code block (returns null). */
function mdMermaid(code) {
  const lines = code.split('\n').map(l => l.replace(/%%.*$/, '').trim()).filter(Boolean);
  const head = /^(?:graph|flowchart)(?:\s+(LR|RL|TD|TB|BT))?\s*;?$/i.exec(lines[0] || '');
  if (!head) return null;
  const dir = (head[1] || 'TD').toUpperCase();
  const horizontal = dir === 'LR' || dir === 'RL';
  const nodes = {}, order = [], edges = [];
  const OPEN = [['([', '])', 'stadium'], ['((', '))', 'circle'], ['[(', ')]', 'rect'], ['[[', ']]', 'rect'],
    ['[', ']', 'rect'], ['(', ')', 'round'], ['{', '}', 'diamond']];
  function node(id, label, shape) {
    let n = nodes[id];
    if (!n) { n = nodes[id] = { id, label: id, shape: 'rect' }; order.push(n); }
    if (label !== null) { n.label = label; n.shape = shape; }
    return n;
  }
  function parseNode(s, p) {
    const m = /^\s*([A-Za-z_][\w-]*)/.exec(s.slice(p));
    if (!m) return null;
    p += m[0].length;
    let label = null, shape = 'rect';
    for (const [o, c, sh] of OPEN) {
      if (s.startsWith(o, p)) {
        const j = s.indexOf(c, p + o.length);
        if (j < 0) continue;
        label = s.slice(p + o.length, j).trim().replace(/^"(.*)"$/, '$1');
        shape = sh; p = j + c.length;
        break;
      }
    }
    return [node(m[1], label === null ? null : label.replace(/<br\s*\/?>/gi, '\n'), shape), p];
  }
  function parseEdge(s, p) {
    const rest = s.slice(p);
    let m = /^\s*(<?)(-->|---|-\.->|-\.-|==>|===|--[ox])\s*(?:\|([^|]*)\|)?/.exec(rest);
    let label = '', arrow, style, both = false;
    if (m) { label = (m[3] || '').trim(); arrow = />$|[ox]$/.test(m[2]); style = m[2].indexOf('.') >= 0 ? 'dot' : m[2][0] === '=' ? 'thick' : 'solid'; both = !!m[1]; }
    else if ((m = /^\s*--\s+(.+?)\s+(-->|---)(?=\s|$)/.exec(rest))) { label = m[1]; arrow = m[2] === '-->'; style = 'solid'; }
    else if ((m = /^\s*-\.\s+(.+?)\s+\.->/.exec(rest))) { label = m[1]; arrow = true; style = 'dot'; }
    else if ((m = /^\s*==\s+(.+?)\s+==>/.exec(rest))) { label = m[1]; arrow = true; style = 'thick'; }
    else return null;
    return [{ label: label.replace(/^"(.*)"$/, '$1'), arrow, style, both }, p + m[0].length];
  }
  for (let li = 1; li < lines.length; li++) {
    const line = lines[li].replace(/;$/, '');
    if (/^(subgraph|end|style|classDef|class|linkStyle|click|direction)\b/i.test(line)) continue;
    const r = parseNode(line, 0);
    if (!r) continue;
    let prev = r[0], p = r[1];
    for (;;) {
      const e = parseEdge(line, p);
      if (!e) break;
      const nx = parseNode(line, e[1]);
      if (!nx) break;
      edges.push(Object.assign({ from: prev.id, to: nx[0].id }, e[0]));
      prev = nx[0]; p = nx[1];
    }
  }
  if (!order.length) return null;

  // layers: longest path over the edges that do not close a cycle
  const out = {}, state = {}, back = new Set();
  order.forEach(n => out[n.id] = []);
  edges.forEach((e, k) => out[e.from].push(k));
  function dfs(id) {
    state[id] = 1;
    for (const k of out[id]) {
      const t = edges[k].to;
      if (state[t] === 1) back.add(k);
      else if (!state[t]) dfs(t);
    }
    state[id] = 2;
  }
  const hasIn = {};
  edges.forEach(e => { if (e.from !== e.to) hasIn[e.to] = true; });
  order.filter(n => !hasIn[n.id]).concat(order).forEach(n => { if (!state[n.id]) dfs(n.id); });
  const rank = {};
  order.forEach(n => rank[n.id] = 0);
  for (let pass = 0; pass < order.length; pass++) {
    let changed = false;
    edges.forEach((e, k) => {
      if (back.has(k) || e.from === e.to) return;
      if (rank[e.to] < rank[e.from] + 1) { rank[e.to] = rank[e.from] + 1; changed = true; }
    });
    if (!changed) break;
  }
  const layers = [];
  order.forEach(n => { (layers[rank[n.id]] = layers[rank[n.id]] || []).push(n); });
  order.forEach(n => {
    n.lines = n.label.split('\n');
    const tw = Math.max(...n.lines.map(l => l.length)) * 7 + 24;
    const th = n.lines.length * 15 + 16;
    n.w = n.shape === 'diamond' ? tw + 28 : n.shape === 'circle' ? Math.max(tw, th) + 6 : Math.max(tw, 52);
    n.h = n.shape === 'diamond' ? th + 18 : n.shape === 'circle' ? Math.max(tw, th) + 6 : Math.max(th, 30);
  });
  // inside a layer, the nodes sit where the ones before them do
  const pos = {};
  layers.forEach((L, li) => {
    if (li > 0) {
      L.forEach(n => {
        const ps = edges.filter((e, k) => e.to === n.id && !back.has(k) && pos[e.from] !== undefined).map(e => pos[e.from]);
        n.bary = ps.length ? ps.reduce((a, b) => a + b, 0) / ps.length : 0;
      });
      L.sort((a, b) => a.bary - b.bary);
    }
    L.forEach((n, k) => pos[n.id] = k);
  });
  const GAP_ALONG = 46, GAP_ACROSS = 28;
  const along = n => horizontal ? n.w : n.h, across = n => horizontal ? n.h : n.w;
  const layerAlong = layers.map(L => Math.max(...L.map(along)));
  const layerAcross = layers.map(L => L.reduce((a, n) => a + across(n), 0) + GAP_ACROSS * (L.length - 1));
  const maxAcross = Math.max(...layerAcross);
  let a0 = 14;
  layers.forEach((L, li) => {
    let c0 = 14 + (maxAcross - layerAcross[li]) / 2;
    L.forEach(n => {
      const ca = a0 + layerAlong[li] / 2, cc = c0 + across(n) / 2;
      n.x = horizontal ? ca : cc; n.y = horizontal ? cc : ca;
      c0 += across(n) + GAP_ACROSS;
    });
    a0 += layerAlong[li] + GAP_ALONG;
  });
  const totalAlong = a0 - GAP_ALONG + 14, totalAcross = maxAcross + 28;
  // room on the far side for the arcs of edges that run back
  const W = (horizontal ? totalAlong : totalAcross) + (back.size && !horizontal ? 60 : 0);
  const H = (horizontal ? totalAcross : totalAlong) + (back.size && horizontal ? 60 : 0);
  if (dir === 'RL') order.forEach(n => { n.x = W - n.x; });
  if (dir === 'BT') order.forEach(n => { n.y = H - n.y; });

  const id = 'mm' + (++MD_DIAGRAMS);
  let svg = '<svg xmlns="http://www.w3.org/2000/svg" width="' + Math.ceil(W) + '" height="' + Math.ceil(H) + '" viewBox="0 0 ' + Math.ceil(W) + ' ' + Math.ceil(H) + '">' +
    '<defs><marker id="' + id + 'a" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M0,0 L10,5 L0,10 z" class="mm-head"/></marker></defs>';
  // the point on a node's outline towards (tx, ty)
  function edgePoint(n, tx, ty) {
    const dx = tx - n.x, dy = ty - n.y;
    if (!dx && !dy) return [n.x, n.y];
    let t;
    if (n.shape === 'circle') t = (n.w / 2) / Math.hypot(dx, dy);
    else if (n.shape === 'diamond') t = 1 / (Math.abs(dx) / (n.w / 2) + Math.abs(dy) / (n.h / 2));
    else t = Math.min(dx ? (n.w / 2) / Math.abs(dx) : Infinity, dy ? (n.h / 2) / Math.abs(dy) : Infinity);
    return [n.x + dx * t, n.y + dy * t];
  }
  edges.forEach((e, k) => {
    const a = nodes[e.from], b = nodes[e.to];
    let d, lx, ly;
    if (a === b) {
      const x = a.x + a.w / 2, y = a.y;
      d = 'M' + x + ',' + (y - 6) + ' C' + (x + 34) + ',' + (y - 24) + ' ' + (x + 34) + ',' + (y + 24) + ' ' + x + ',' + (y + 6);
      lx = x + 30; ly = y;
    } else if (back.has(k)) {
      // out of the side and round, so it does not run through the nodes in between
      const bulge = 44;
      if (horizontal) {
        const y0 = a.y + a.h / 2, y1 = b.y + b.h / 2, yy = Math.max(y0, y1) + bulge;
        d = 'M' + a.x + ',' + y0 + ' C' + a.x + ',' + yy + ' ' + b.x + ',' + yy + ' ' + b.x + ',' + y1;
        lx = (a.x + b.x) / 2; ly = yy - 8;
      } else {
        const x0 = a.x + a.w / 2, x1 = b.x + b.w / 2, xx = Math.max(x0, x1) + bulge;
        d = 'M' + x0 + ',' + a.y + ' C' + xx + ',' + a.y + ' ' + xx + ',' + b.y + ' ' + x1 + ',' + b.y;
        lx = xx - 8; ly = (a.y + b.y) / 2;
      }
    } else {
      const p0 = edgePoint(a, b.x, b.y), p1 = edgePoint(b, a.x, a.y);
      d = horizontal
        ? 'M' + p0[0] + ',' + p0[1] + ' C' + (p0[0] + p1[0]) / 2 + ',' + p0[1] + ' ' + (p0[0] + p1[0]) / 2 + ',' + p1[1] + ' ' + p1[0] + ',' + p1[1]
        : 'M' + p0[0] + ',' + p0[1] + ' C' + p0[0] + ',' + (p0[1] + p1[1]) / 2 + ' ' + p1[0] + ',' + (p0[1] + p1[1]) / 2 + ' ' + p1[0] + ',' + p1[1];
      lx = (p0[0] + p1[0]) / 2; ly = (p0[1] + p1[1]) / 2;
    }
    svg += '<path d="' + d + '" class="mm-edge' + (e.style === 'dot' ? ' dot' : e.style === 'thick' ? ' thick' : '') + '"' +
      (e.arrow ? ' marker-end="url(#' + id + 'a)"' : '') + (e.both ? ' marker-start="url(#' + id + 'a)"' : '') + '/>';
    if (e.label) {
      const tw = e.label.length * 6.4 + 10;
      svg += '<rect x="' + (lx - tw / 2) + '" y="' + (ly - 9) + '" width="' + tw + '" height="18" rx="3" class="mm-lbl"/>' +
        '<text x="' + lx + '" y="' + ly + '" class="mm-text sm" text-anchor="middle" dominant-baseline="central">' + escapeHtml(e.label) + '</text>';
    }
  });
  order.forEach(n => {
    const x = n.x - n.w / 2, y = n.y - n.h / 2;
    if (n.shape === 'diamond') svg += '<polygon points="' + n.x + ',' + y + ' ' + (x + n.w) + ',' + n.y + ' ' + n.x + ',' + (y + n.h) + ' ' + x + ',' + n.y + '" class="mm-node"/>';
    else if (n.shape === 'circle') svg += '<circle cx="' + n.x + '" cy="' + n.y + '" r="' + n.w / 2 + '" class="mm-node"/>';
    else svg += '<rect x="' + x + '" y="' + y + '" width="' + n.w + '" height="' + n.h + '" rx="' + (n.shape === 'stadium' ? n.h / 2 : n.shape === 'round' ? 8 : 3) + '" class="mm-node"/>';
    const top = n.y - (n.lines.length - 1) * 7.5;
    n.lines.forEach((l, k) => {
      svg += '<text x="' + n.x + '" y="' + (top + k * 15) + '" class="mm-text" text-anchor="middle" dominant-baseline="central">' + escapeHtml(l) + '</text>';
    });
  });
  return '<div class="md-mermaid">' + svg + '</svg></div>';
}

/* ---------- blocks ---------- */
const MD_FENCE = /^(\s{0,3})(`{3,}|~{3,})\s*([^`\s]*)[^`]*$/;
const MD_HR = /^\s{0,3}([-*_])(\s*\1){2,}\s*$/;
const MD_ATTR_LINE = /^\{:\s*(.*?)\s*\}\s*$/;
const MD_ITEM = /^(\s*)([-*+]|\d{1,9}[.)])(\s+|$)(.*)$/;
const MD_HTML_BLOCK = /^\s*<(details|div)(\s[^>]*)?>\s*$/i;

function mdListMarker(line) { return MD_ITEM.exec(line); }
function mdIsTable(lines, i) {
  return i + 1 < lines.length && /\|/.test(lines[i]) &&
    /^\s*\|?\s*:?-+:?\s*(\|\s*:?-+:?\s*)+\|?\s*$/.test(lines[i + 1]);
}
function mdIsBlockStart(lines, i) {
  const line = lines[i];
  return /^\s*$/.test(line)
    || MD_FENCE.test(line)
    || /^\s*\$\$/.test(line)
    || /^:::/.test(line)
    || MD_HTML_BLOCK.test(line)
    || /^\s*(\[\[toc\]\]|\[TOC\])\s*$/i.test(line)
    || /^\s{0,3}#{1,6}(\s|$)/.test(line)
    || /^\s{0,3}>/.test(line)
    || MD_HR.test(line)
    || !!mdListMarker(line)
    || mdIsTable(lines, i);
}
function mdSplitRow(line) {
  return line.trim().replace(/^\|/, '').replace(/\|$/, '')
    .replace(/\\\|/g, '\u0001').split('|')
    .map(c => c.trim().replace(/\u0001/g, '|'));
}
function mdIndent(line) { return /^\s*/.exec(line)[0].replace(/\t/g, '    ').length; }
/* The id and classes of "{: .class #id }". */
function mdAttrs(spec, idIn) {
  let id = idIn || null; const cls = [];
  spec.split(/\s+/).forEach(a => {
    if (a[0] === '.') cls.push(a.slice(1).replace(/[^\w-]/g, ''));
    else if (a[0] === '#') id = a.slice(1).replace(/[^\w-]/g, '');
  });
  return { id, cls: cls.join(' ') };
}
function mdSlug(text) {
  let s = text.toLowerCase().trim().replace(/[^\w\s-]/g, '').replace(/\s+/g, '-');
  if (!s) s = 'section';
  const n = MD.ids[s] = (MD.ids[s] || 0) + 1;
  return n > 1 ? s + '-' + (n - 1) : s;
}
function mdHeading(level, raw) {
  let id = null, cls = '';
  raw = raw.replace(/\s*\{#([\w-]+)\}\s*$/, (m, x) => { id = x; return ''; });
  raw = raw.replace(/\s*\{:\s*([^}]*)\}\s*$/, (m, spec) => { const a = mdAttrs(spec, id); id = a.id; cls = a.cls; return ''; });
  const html = mdInline(raw);
  const plain = html.replace(/<[^>]*>/g, '').replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&quot;/g, '"').replace(/&amp;/g, '&');
  if (id) MD.ids[id.toLowerCase()] = (MD.ids[id.toLowerCase()] || 0) + 1; else id = mdSlug(plain);
  MD.headings.push({ level, id, text: html });
  return '<h' + level + ' id="' + id + '"' + (cls ? ' class="' + cls + '"' : '') + '>' + html + '</h' + level + '>';
}
function mdCodeBlock(lang, code, closed) {
  const l = (lang || '').toLowerCase();
  if (l === 'mermaid' && closed) {
    const svg = mdMermaid(code);
    if (svg) return svg;
  }
  if ((l === 'math' || l === 'latex' || l === 'tex') && closed) return mdMath(code, true);
  let body;
  if (l === 'diff' || l === 'patch') {
    body = code.split('\n').map(line => {
      const c = /^@@/.test(line) ? 'df-hunk' : /^(\+\+\+|---) /.test(line) ? 'df-file' : line[0] === '+' ? 'df-add' : line[0] === '-' ? 'df-del' : '';
      return c ? '<span class="' + c + '">' + escapeHtml(line) + '</span>' : escapeHtml(line);
    }).join('\n');
  } else body = escapeHtml(code);
  return '<pre><div class="cb-head"><span>' + escapeHtml(lang || 'code') +
    '</span><button title="Copy" onclick="copyCode(this)">' + ICONS.COPY + '</button></div><code>' + body + '</code></pre>';
}
function mdParseTable(lines, start) {
  const header = mdSplitRow(lines[start]);
  const align = mdSplitRow(lines[start + 1]).map(c => /^:-*:$/.test(c) ? 'center' : /^-*:$/.test(c) ? 'right' : /^:-*$/.test(c) ? 'left' : '');
  const style = k => align[k] ? ' style="text-align:' + align[k] + '"' : '';
  let i = start + 2, out = '<table><thead><tr>';
  header.forEach((h, k) => out += '<th' + style(k) + '>' + mdInline(h) + '</th>');
  out += '</tr></thead><tbody>';
  while (i < lines.length && /\|/.test(lines[i]) && !/^\s*$/.test(lines[i])) {
    const cells = mdSplitRow(lines[i]);
    out += '<tr>';
    for (let c = 0; c < header.length; c++) out += '<td' + style(c) + '>' + mdInline(cells[c] || '') + '</td>';
    out += '</tr>';
    i++;
  }
  return [out + '</tbody></table>', i];
}
function mdItemKind(marker) { return /\d/.test(marker) ? marker.slice(-1) : marker; }
/* A list starting at `start`; returns [html, nextIndex]. */
function mdParseList(lines, start) {
  const first = MD_ITEM.exec(lines[start]);
  const base = first[1].length;
  const ordered = /\d/.test(first[2]);
  const kind = mdItemKind(first[2]);
  const startNum = ordered ? parseInt(first[2], 10) : 1;
  const items = [];
  let i = start, loose = false;
  while (i < lines.length) {
    const m = MD_ITEM.exec(lines[i]);
    if (!m || m[1].length !== base) break;
    if (mdItemKind(m[2]) !== kind) break;   // a different bullet starts another list
    const gap = m[3].length >= 1 && m[3].length <= 4 && m[4] !== '' ? m[3].length : 1;
    const contentIndent = base + m[2].length + gap;
    const body = [m[4]];
    let sawBlank = false, itemLoose = false, fences = 0;
    i++;
    if (/^(`{3,}|~{3,})/.test(m[4].trim())) fences++;
    while (i < lines.length) {
      const L = lines[i];
      if (/^\s*$/.test(L)) {
        let j = i;
        while (j < lines.length && /^\s*$/.test(lines[j])) j++;
        if (j < lines.length && mdIndent(lines[j]) > base) {
          if (fences % 2 === 0) itemLoose = true;
          for (; i < j; i++) body.push('');
          continue;
        }
        sawBlank = j < lines.length;
        break;
      }
      const ind = mdIndent(L);
      if (ind > base) {
        const strip = Math.min(ind, contentIndent);
        body.push(L.replace(/\t/g, '    ').slice(strip));
        if (/^\s*(`{3,}|~{3,})/.test(L)) fences++;
        i++;
        continue;
      }
      // a line at the list's own margin that is no item or block of its own carries on the text above
      if (!mdIsBlockStart(lines, i) && !/^\s*$/.test(body[body.length - 1]) && fences % 2 === 0) {
        body.push(L.trim()); i++; continue;
      }
      break;
    }
    if (itemLoose) loose = true;
    items.push(body);
    if (sawBlank) {
      // blank lines before another item of this list make it loose; before anything else they end it
      let j = i;
      while (j < lines.length && /^\s*$/.test(lines[j])) j++;
      const nm = j < lines.length && MD_ITEM.exec(lines[j]);
      if (nm && nm[1].length === base && mdItemKind(nm[2]) === kind) { loose = true; i = j; }
      else break;
    }
  }
  const tag = ordered ? 'ol' : 'ul';
  let out = '<' + tag + (ordered && startNum !== 1 ? ' start="' + startNum + '"' : '') + '>';
  items.forEach(body => {
    let cls = '', box = '';
    const tm = /^\[([ xX])\]\s+([\s\S]*)$/.exec(body[0]);
    if (tm) {
      cls = ' class="task"';
      box = '<input type="checkbox" disabled' + (/[xX]/.test(tm[1]) ? ' checked' : '') + '> ';
      body = [tm[2]].concat(body.slice(1));
    }
    out += '<li' + cls + '>' + box + mdRenderBlocks(body, !loose) + '</li>';
  });
  return [out + '</' + tag + '>', i];
}
/* The line that closes the `tag` opened at `start` (same tag nested inside is counted); -1 if none yet. */
function mdHtmlBlockEnd(lines, start, tag) {
  const open = new RegExp('<' + tag + '[\\s>]', 'gi'), close = new RegExp('</' + tag + '\\s*>', 'gi');
  let depth = 0;
  for (let i = start; i < lines.length; i++) {
    depth += (lines[i].match(open) || []).length - (lines[i].match(close) || []).length;
    if (depth <= 0) return i;
  }
  return -1;
}
/* Block-level parse over a slice of lines. `tight`: paragraphs are bare text (a tight list item). */
function mdRenderBlocks(lines, tight) {
  let html = '', i = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (/^\s*$/.test(line)) { i++; continue; }

    let m = MD_FENCE.exec(line);
    if (m) {
      const ch = m[2][0], len = m[2].length, indent = m[1].length;
      const closeRe = new RegExp('^\\s{0,3}' + (ch === '`' ? '`' : '~') + '{' + len + ',}\\s*$');
      const code = [];
      i++;
      while (i < lines.length && !closeRe.test(lines[i])) {
        code.push(lines[i].replace(new RegExp('^ {0,' + indent + '}'), '')); i++;
      }
      const closed = i < lines.length;
      if (closed) i++;
      html += mdCodeBlock(m[3], code.join('\n'), closed);
      continue;
    }
    if (/^\s*\$\$/.test(line)) {
      const one = /^\s*\$\$(.+?)\$\$\s*$/.exec(line);
      if (one) { html += mdMath(one[1], true); i++; continue; }
      const tex = [line.replace(/^\s*\$\$/, '')];
      i++;
      while (i < lines.length && !/\$\$\s*$/.test(lines[i])) { tex.push(lines[i]); i++; }
      if (i < lines.length) { tex.push(lines[i].replace(/\$\$\s*$/, '')); i++; }
      html += mdMath(tex.join('\n'), true);
      continue;
    }
    if (!/^:::\s*$/.test(line) && (m = /^:::\s*(\w*)\s*(.*)$/.exec(line))) {
      const type = (m[1] || 'note').toLowerCase().replace(/[^\w-]/g, '');
      const title = m[2].trim() || type.charAt(0).toUpperCase() + type.slice(1);
      const inner = [];
      i++;
      while (i < lines.length && !/^:::\s*$/.test(lines[i])) { inner.push(lines[i]); i++; }
      if (i < lines.length) i++;
      html += '<div class="md-box md-box-' + type + '"><div class="md-box-title">' + mdInline(title) + '</div>' + mdRenderBlocks(inner, false) + '</div>';
      continue;
    }
    if ((m = MD_HTML_BLOCK.exec(line))) {
      const tag = m[1].toLowerCase();
      const end = mdHtmlBlockEnd(lines, i, tag);
      const inner = lines.slice(i + 1, end < 0 ? lines.length : end);
      const attrs = m[2] || '';
      i = end < 0 ? lines.length : end + 1;
      if (tag === 'details') {
        let summary = '';
        const k = inner.findIndex(l => /\S/.test(l));
        if (k >= 0) {
          const sm = /^\s*<summary[^>]*>([\s\S]*?)<\/summary>\s*$/i.exec(inner[k]);
          if (sm) { summary = sm[1]; inner.splice(k, 1); }
        }
        html += '<details' + (/\sopen\b/i.test(attrs) ? ' open' : '') + '><summary>' + mdInline(summary || 'Details') + '</summary>' + mdRenderBlocks(inner, false) + '</details>';
      } else {
        const al = /align\s*=\s*"?(center|right|left)/i.exec(attrs);
        html += '<div' + (al ? ' class="md-align-' + al[1].toLowerCase() + '"' : '') + '>' + mdRenderBlocks(inner, false) + '</div>';
      }
      continue;
    }
    if (/^\s*<!--.*-->\s*$/.test(line)) { i++; continue; }
    if (/^\s*<hr\s*\/?>\s*$/i.test(line)) { html += '<hr>'; i++; continue; }
    if (/^\s*(\[\[toc\]\]|\[TOC\])\s*$/i.test(line)) { MD.toc = true; html += '\u0000TOC\u0000'; i++; continue; }
    if (MD_HR.test(line)) { html += '<hr>'; i++; continue; }
    if ((m = /^\s{0,3}(#{1,6})(?:\s+(.*?))?(?:\s+#+)?\s*$/.exec(line))) {
      html += mdHeading(m[1].length, m[2] || ''); i++; continue;
    }
    if (/^\s{0,3}>/.test(line)) {
      const buf = [];
      while (i < lines.length && /^\s{0,3}>/.test(lines[i])) { buf.push(lines[i].replace(/^\s{0,3}>\s?/, '')); i++; }
      const am = /^\[!(NOTE|TIP|IMPORTANT|WARNING|CAUTION)\]\s*$/i.exec(buf[0] || '');
      if (am) {
        const kind = am[1].toLowerCase();
        html += '<div class="md-alert md-alert-' + kind + '"><div class="md-alert-title">' + kind.charAt(0).toUpperCase() + kind.slice(1) + '</div>' + mdRenderBlocks(buf.slice(1), false) + '</div>';
      } else html += '<blockquote>' + mdRenderBlocks(buf, false) + '</blockquote>';
      continue;
    }
    if (mdIsTable(lines, i)) { const r = mdParseTable(lines, i); html += r[0]; i = r[1]; continue; }
    if (mdListMarker(line)) { const r = mdParseList(lines, i); html += r[0]; i = r[1]; continue; }
    if (/^( {4}|\t)/.test(line)) {
      const code = [];
      while (i < lines.length) {
        if (/^( {4}|\t)/.test(lines[i])) { code.push(lines[i].replace(/^( {4}|\t)/, '')); i++; }
        else if (/^\s*$/.test(lines[i]) && i + 1 < lines.length && /^( {4}|\t)/.test(lines[i + 1])) { code.push(''); i++; }
        else break;
      }
      html += mdCodeBlock('', code.join('\n'), true);
      continue;
    }
    if (i + 1 < lines.length && /^\s{0,3}:\s+\S/.test(lines[i + 1])) {
      let dl = '<dl>';
      for (;;) {
        dl += '<dt>' + mdInline(lines[i].trim()) + '</dt>'; i++;
        while (i < lines.length && /^\s{0,3}:\s+\S/.test(lines[i])) { dl += '<dd>' + mdInline(lines[i].replace(/^\s{0,3}:\s+/, '').trim()) + '</dd>'; i++; }
        let j = i;
        while (j < lines.length && /^\s*$/.test(lines[j])) j++;
        // another term, if the line after it is a definition
        if (j + 1 < lines.length && !/^\s{0,3}:\s/.test(lines[j]) && /^\s{0,3}:\s+\S/.test(lines[j + 1])) i = j; else break;
      }
      html += dl + '</dl>';
      continue;
    }
    // paragraph (or a setext heading)
    const buf = [];
    let attr = null, heading = 0;
    while (i < lines.length) {
      buf.push(lines[i]); i++;
      if (i < lines.length) {
        if (/^\s{0,3}=+\s*$/.test(lines[i])) { heading = 1; i++; break; }
        if (/^\s{0,3}-{2,}\s*$/.test(lines[i])) { heading = 2; i++; break; }
        const am = MD_ATTR_LINE.exec(lines[i].trim());
        if (am) { attr = mdAttrs(am[1]); i++; break; }
      }
      if (i >= lines.length || mdIsBlockStart(lines, i)) break;
    }
    if (heading) { html += mdHeading(heading, buf.join(' ')); continue; }
    const inline = mdInline(buf.join('\n')).replace(/\n/g, '<br>');
    if (tight && !attr) html += inline;
    else html += '<p' + (attr && attr.id ? ' id="' + attr.id + '"' : '') + (attr && attr.cls ? ' class="' + attr.cls + '"' : '') + '>' + inline + '</p>';
  }
  return html;
}

/* A front matter block at the very top is shown as a yaml code block. */
function mdFrontMatter(lines) {
  if (lines[0] !== '---' || lines.length < 3 || !/^[\w.-]+\s*:/.test(lines[1])) return lines;
  for (let j = 2; j < lines.length; j++) {
    if (lines[j] === '---' || lines[j] === '...') {
      return ['```yaml'].concat(lines.slice(1, j), ['```'], lines.slice(j + 1));
    }
  }
  return lines;
}
/* Takes the definitions (links, footnotes, abbreviations) out of the text and into MD. */
function mdCollectDefs(lines) {
  const out = [];
  let fence = null;
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const f = /^\s{0,3}(`{3,}|~{3,})/.exec(line);
    if (fence) {
      if (f && f[1][0] === fence[0] && f[1].length >= fence.length && /^\s*(`+|~+)\s*$/.test(line)) fence = null;
      out.push(line); continue;
    }
    if (f) { fence = f[1]; out.push(line); continue; }
    let m;
    if ((m = /^\s{0,3}\[\^([^\]\s]+)\]:\s*(.*)$/.exec(line))) {
      let text = m[2];
      while (i + 1 < lines.length && /^( {2,}|\t)\S/.test(lines[i + 1])) { text += ' ' + lines[i + 1].trim(); i++; }
      MD.footnotes[m[1]] = text;
      continue;
    }
    if ((m = /^\*\[([^\]]+)\]:\s*(.+)$/.exec(line))) { MD.abbrs[m[1]] = escapeHtml(m[2].trim()); continue; }
    if ((m = /^\s{0,3}\[([^\]^][^\]]*)\]:\s*<?(\S+?)>?(?:\s+(?:"(.*?)"|'(.*?)'|\((.*?)\)))?\s*$/.exec(line))) {
      MD.refs[m[1].toLowerCase()] = { url: escapeHtml(m[2]), title: escapeHtml(m[3] || m[4] || m[5] || '') };
      continue;
    }
    out.push(line);
  }
  return out;
}
function mdToc() {
  const hs = MD.headings;
  if (!hs.length) return '';
  const min = Math.min(...hs.map(h => h.level));
  let out = '<nav class="md-toc">', depth = 0;
  hs.forEach(h => {
    const d = h.level - min + 1;
    while (depth < d) { out += '<ul>'; depth++; }
    while (depth > d) { out += '</ul>'; depth--; }
    out += '<li><a href="#' + h.id + '">' + h.text + '</a></li>';
  });
  while (depth > 0) { out += '</ul>'; depth--; }
  return out + '</nav>';
}
function renderMarkdown(text) {
  if (!text) return '';
  const prev = MD;
  MD = mdNewCtx();
  try {
    let lines = text.replace(/\r\n?/g, '\n').replace(/\u0000/g, '').split('\n');
    lines = mdCollectDefs(mdFrontMatter(lines));
    let html = mdRenderBlocks(lines, false);
    if (MD.toc) html = html.replace(/\u0000TOC\u0000/g, mdToc());
    if (MD.fnOrder.length) {
      let fn = '<section class="md-footnotes"><hr><ol>';
      for (let k = 0; k < MD.fnOrder.length; k++) {   // may grow: a note can refer to another
        const id = MD.fnOrder[k], fid = id.replace(/[^\w-]/g, '_');
        fn += '<li id="fn-' + fid + '">' + mdInline(MD.footnotes[id]) + ' <a class="fn-back" href="#fnref-' + fid + '">↩</a></li>';
      }
      html += fn + '</ol></section>';
    }
    return html;
  } finally { MD = prev; }
}
/* Through chat.js's copyToClipboard (native _clipSet bridge first) rather than
   navigator.clipboard alone: macOS's WKWebView rejects writeText on this page's
   non-secure origin, so the fence's Copy silently did nothing there while
   WebView2/WebKitGTK allowed it. */
function copyCode(btn) {
  copyToClipboard(btn, btn.closest('pre').querySelector('code').textContent);
}
