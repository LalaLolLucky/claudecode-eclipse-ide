use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// Compute the Claude CLI project hash for a workspace path.
///
/// The algorithm mirrors what Claude CLI uses: every character that is not
/// ASCII alphanumeric becomes `-` (so `:`, `\`, `/`, spaces, dots, etc. all map
/// to `-`). Example:
///   `C:\Users\Windows 10\Project` → `C--Users-Windows-10-Project`
/// Replacing only `:\/` (the previous behaviour) broke any path containing a
/// space — e.g. the "Windows 10" home folder — so no sessions were ever found.
pub(crate) fn workspace_hash(workspace_root: &str) -> String {
    workspace_root
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Strip the editor-context preamble AND Claude Code command/meta wrappers so
/// session titles aren't raw `<ide_selection>` / `<command-name>` /
/// `<local-command-*>` tags. Removes ALL occurrences, not just a leading one,
/// and unwraps `<command-name>X</command-name>` to `X`. No regex crate needed —
/// simple scans.
fn strip_ide_preamble(s: &str) -> String {
    let mut t = remove_tag_block(s, "ide_selection");
    t = remove_self_closing_tag(&t, "ide_context");
    // Arrived with CLI 2.1.x as a text block prepended to the user's own message,
    // so without this a session title (and the delete sweep's text match) starts
    // with a paragraph about which file was open.
    t = remove_tag_block(&t, "ide_opened_file");
    t = remove_tag_block(&t, "local-command-caveat");
    t = remove_tag_block(&t, "command-message");
    t = remove_tag_block(&t, "command-args");
    t = remove_tag_block(&t, "local-command-stdout");
    t = unwrap_tag_block(&t, "command-name");
    t.trim_start().to_string()
}

/// A synthetic message the host sent (the browser-disconnected notice), as the CLI
/// records it: flagged `isMeta`, its text prefixed with this marker (CLI 2.1.266).
const NON_USER_MARKER: &str = "[MESSAGE FROM NON-USER SOURCE - NOT USER INPUT]";

fn is_non_user_notice(event: &serde_json::Value, content: &str) -> bool {
    event["isMeta"].as_bool().unwrap_or(false) && content.starts_with(NON_USER_MARKER)
}

/// A background-task notification the host injected while the conversation ran.
///
/// The CLI stores it as an ordinary user line whose whole text is the
/// `<task-notification>` block (and, in its queued form, carries
/// `commandMode: "task-notification"`). Nobody typed it, so reopening the
/// conversation must not show it as something the user said — which is exactly how
/// it looked: several raw XML blocks in a row where the messages should be.
fn is_task_notification(event: &serde_json::Value, content: &str) -> bool {
    const TAG: &str = "<task-notification>";
    const MODE: &str = "task-notification";
    content.trim_start().starts_with(TAG)
        || event["commandMode"].as_str() == Some(MODE)
        || event["attachment"]["commandMode"].as_str() == Some(MODE)
}

/// Finds the next `<tag ...>` opening (word-boundary after the tag name, like
/// `\b` in a regex) at or after byte `from`. Returns (start, end-of-open-tag)
/// byte offsets, the end being one past the closing `>`.
fn find_open_tag(s: &str, tag: &str, from: usize) -> Option<(usize, usize)> {
    let pat = format!("<{}", tag);
    let mut i = from;
    while let Some(rel) = s[i..].find(&pat) {
        let start = i + rel;
        let after = start + pat.len();
        let boundary = s[after..]
            .chars()
            .next()
            .map_or(false, |c| c == '>' || c == '/' || c.is_whitespace());
        if boundary {
            if let Some(gt) = s[after..].find('>') {
                return Some((start, after + gt + 1));
            }
            return None; // unterminated open tag — nothing to strip
        }
        i = after;
    }
    None
}

/// Removes every `<tag ...>inner</tag>` block, inner included. A block missing
/// its closing tag is left untouched.
fn remove_tag_block(s: &str, tag: &str) -> String {
    let close = format!("</{}>", tag);
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while let Some((start, open_end)) = find_open_tag(s, tag, i) {
        match s[open_end..].find(&close) {
            Some(rel) => {
                out.push_str(&s[i..start]);
                i = open_end + rel + close.len();
            }
            None => break,
        }
    }
    out.push_str(&s[i..]);
    out
}

/// Removes every self-closing `<tag ... />` occurrence.
fn remove_self_closing_tag(s: &str, tag: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while let Some((start, open_end)) = find_open_tag(s, tag, i) {
        if s[..open_end].ends_with("/>") {
            out.push_str(&s[i..start]);
        } else {
            out.push_str(&s[i..open_end]);
        }
        i = open_end;
    }
    out.push_str(&s[i..]);
    out
}

/// Replaces every `<tag>inner</tag>` with just `inner`.
fn unwrap_tag_block(s: &str, tag: &str) -> String {
    let close = format!("</{}>", tag);
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while let Some((start, open_end)) = find_open_tag(s, tag, i) {
        match s[open_end..].find(&close) {
            Some(rel) => {
                out.push_str(&s[i..start]);
                out.push_str(&s[open_end..open_end + rel]);
                i = open_end + rel + close.len();
            }
            None => break,
        }
    }
    out.push_str(&s[i..]);
    out
}

/// Returns the path to `~/.claude/projects/{hash}/`.
fn projects_dir(workspace_root: &str) -> Option<PathBuf> {
    let home = dirs_home()?;
    let hash = workspace_hash(workspace_root);
    let dir = home.join(".claude").join("projects").join(hash);
    if dir.is_dir() {
        Some(dir)
    } else {
        None
    }
}

/// Formats a Unix epoch-seconds value as an ISO-8601 UTC string
/// (`YYYY-MM-DDTHH:MM:SSZ`) using the civil-from-days algorithm (Howard Hinnant's,
/// public domain) — no external crate. Only used as a sort-key fallback for the rare
/// title-only session stubs whose events carry no timestamp, so it string-sorts
/// interleaved with the real ISO timestamps from normal sessions.
fn epoch_to_iso8601(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (hh, mm, ss) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // Days since 1970-01-01 → civil (year, month, day).
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as i64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = if m <= 2 { y + 1 } else { y };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, m, d, hh, mm, ss
    )
}

/// Platform-agnostic home directory lookup.
pub(crate) fn dirs_home() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var("USERPROFILE").ok().map(PathBuf::from)
    }
    #[cfg(not(windows))]
    {
        std::env::var("HOME").ok().map(PathBuf::from)
    }
}

// ---------------------------------------------------------------------------
// list_sessions  — scan *.jsonl files, extract first user message + timestamp
// ---------------------------------------------------------------------------

pub fn list_sessions(workspace_root: &str) -> String {
    let dir = match projects_dir(workspace_root) {
        Some(d) => d,
        None => return "[]".into(),
    };

    let mut sessions: Vec<serde_json::Value> = Vec::new();

    let entries: Vec<_> = match fs::read_dir(&dir) {
        Ok(rd) => rd.filter_map(|e| e.ok()).collect(),
        Err(_) => return "[]".into(),
    };

    for entry in entries {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }

        let session_id = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };

        // Read just enough of the file to find the first user message and timestamp.
        let file = match fs::File::open(&path) {
            Ok(f) => f,
            Err(_) => continue,
        };
        let reader = BufReader::new(file);

        // The list title mirrors the CLI's /resume: the user's rename ("custom-title"
        // event, written by the rename_session control request — LAST one wins) beats
        // the AI-generated "ai-title", which beats the first user message. Neither
        // title event carries a timestamp. The legacy Eclipse-only rename sidecar
        // (session-titles.json, applied Java-side) still overrides all of these.
        // Sort key is the LAST activity timestamp (newest event scanned), matching
        // /resume's most-recently-used ordering.
        let mut custom_title = String::new();
        let mut ai_title = String::new();
        let mut first_user = String::new();
        let mut last_ts = String::new();
        let mut saw_line = false;

        for line in reader.lines() {
            let line = match line {
                Ok(l) if !l.is_empty() => l,
                _ => continue,
            };
            let event: serde_json::Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(_) => continue,
            };
            saw_line = true;

            if let Some(ts) = event["timestamp"].as_str() {
                last_ts = ts.to_string();
            }
            match event["type"].as_str() {
                Some("custom-title") => {
                    if let Some(ct) = event["customTitle"].as_str() {
                        if !ct.is_empty() {
                            custom_title = ct.chars().take(120).collect();
                        }
                    }
                }
                Some("ai-title") => {
                    if let Some(at) = event["aiTitle"].as_str() {
                        if !at.is_empty() {
                            ai_title = at.chars().take(120).collect();
                        }
                    }
                }
                Some("user") if first_user.is_empty() => {
                    // Extract display text — first 120 chars of the user message content,
                    // with any injected <ide_selection>/<ide_context> preamble removed so
                    // the fallback title is the user's actual text, not the editor context.
                    if let Some(content) = event["message"]["content"].as_str() {
                        first_user = strip_ide_preamble(content).chars().take(120).collect();
                    } else if let Some(blocks) = event["message"]["content"].as_array() {
                        // A first message sent with a pasted image is stored as
                        // content blocks — title the session from its text block
                        // instead of falling through to a later message.
                        for b in blocks {
                            if b["type"].as_str() != Some("text") {
                                continue;
                            }
                            let raw = b["text"].as_str().unwrap_or("");
                            if is_browser_context(raw) || attached_file_path(raw).is_some() {
                                continue;
                            }
                            let s = strip_ide_preamble(raw);
                            if !s.trim().is_empty() {
                                first_user = s.chars().take(120).collect();
                                break;
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        // Include a session with any recognizable title source: a custom-title
        // (user rename), an ai-title (covers title-only stubs that /resume lists)
        // or a first user message.
        let display = if !custom_title.is_empty() {
            custom_title
        } else if !ai_title.is_empty() {
            ai_title
        } else {
            first_user
        };
        if !saw_line || display.is_empty() {
            continue;
        }
        // Fall back to file mtime when no event carried a timestamp (e.g. stubs).
        // Format as an ISO-8601 UTC string so it string-sorts interleaved with the
        // real event timestamps (the PHP reader does the same via gmdate()).
        if last_ts.is_empty() {
            if let Ok(meta) = fs::metadata(&path) {
                if let Ok(modified) = meta.modified() {
                    if let Ok(dur) = modified.duration_since(std::time::UNIX_EPOCH) {
                        last_ts = epoch_to_iso8601(dur.as_secs());
                    }
                }
            }
        }

        sessions.push(serde_json::json!({
            "sessionId": session_id,
            "display": display,
            "timestamp": last_ts,
        }));
    }

    // Sort by timestamp descending (newest first).
    sessions.sort_by(|a, b| {
        let ta = a["timestamp"].as_str().unwrap_or("");
        let tb = b["timestamp"].as_str().unwrap_or("");
        tb.cmp(ta)
    });

    // Limit to 100 most recent sessions.
    sessions.truncate(100);

    serde_json::to_string(&sessions).unwrap_or_else(|_| "[]".into())
}

// ---------------------------------------------------------------------------
// search_session_content — grep a caller-supplied subset of sessions for a
// query string, message text only (not titles — the caller already knows how
// to match those instantly from the cached list_sessions result, so it only
// asks this for the sessions whose title didn't match). First hit per file
// wins: the file is read line-by-line and abandoned the moment a match is
// found, so a session's cost is bounded by how early the match falls, not by
// its total length.
//
// Cooperative cancellation: every call publishes its own `generation` as the
// latest one requested (SEARCH_GENERATION), then checks before starting each
// session file whether a NEWER call has since arrived — the caller fires one
// search per keystroke, so a slow typist's Nth keystroke would otherwise still
// be scanning file #1 while the (N+1)th keystroke's results are already what
// the UI wants. A superseded scan exits at the next file boundary rather than
// running to completion for a result the UI is about to discard anyway.
// ---------------------------------------------------------------------------

static SEARCH_GENERATION: AtomicU64 = AtomicU64::new(0);

/// Nearest valid UTF-8 char boundary at or BEFORE `idx` (never past it) — a portable
/// stand-in for the standard library's floor_char_boundary, which is still
/// nightly-only. Used to safely widen/narrow a byte-offset window computed against a
/// DIFFERENT string's positions (see search_session_content's snippet extraction).
fn floor_char_boundary(s: &str, idx: usize) -> usize {
    let mut i = idx.min(s.len());
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Nearest valid UTF-8 char boundary at or AFTER `idx` (never past the string's end).
fn ceil_char_boundary(s: &str, idx: usize) -> usize {
    let mut i = idx.min(s.len());
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

/// @param generation this call's ordinal (the caller increments a per-session-search
///   counter each time the query changes) — used only for cancellation, unrelated to
///   the requestId round-tripped back to JS for discarding stale results.
/// @param own_messages_only restrict the scan to `type:"user"` events (the user's own
///   messages), skipping assistant turns entirely — a cheaper, narrower scope than
///   the full conversation.
pub fn search_session_content(workspace_root: &str, session_ids: &[String], query: &str, own_messages_only: bool, generation: u64) -> String {
    // A plain store, not fetch_max: semantically, every call IS the latest request,
    // full stop — "the latest caller wins" is the actual rule, not "the highest number
    // wins". fetch_max ratcheted this upward forever, so once ANY higher generation had
    // ever been seen, a legitimately newer but lower-numbered request (e.g. after
    // searchRequestId resets to 0 on a webview reload, while this native library and
    // its process-lifetime static stay loaded) could never win again and would silently
    // return zero matches — caught by a test failure whose real cause turned out to be
    // exactly this, not test-order flakiness.
    SEARCH_GENERATION.store(generation, Ordering::Relaxed);

    let dir = match projects_dir(workspace_root) {
        Some(d) => d,
        None => return "[]".into(),
    };
    let needle = query.to_lowercase();
    if needle.is_empty() {
        return "[]".into();
    }

    let mut results: Vec<serde_json::Value> = Vec::new();

    for session_id in session_ids {
        if SEARCH_GENERATION.load(Ordering::Relaxed) != generation {
            break;   // superseded by a newer keystroke's search — stop wasted I/O
        }
        let path = dir.join(format!("{session_id}.jsonl"));
        let file = match fs::File::open(&path) {
            Ok(f) => f,
            Err(_) => continue,
        };
        let reader = BufReader::new(file);

        for line in reader.lines() {
            // Checked every line, not just every file: one large session shouldn't
            // stall a supersede until its whole file is read.
            if SEARCH_GENERATION.load(Ordering::Relaxed) != generation {
                return serde_json::to_string(&results).unwrap_or_else(|_| "[]".into());
            }
            let line = match line {
                Ok(l) if !l.is_empty() => l,
                _ => continue,
            };
            let event: serde_json::Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if own_messages_only && event["type"].as_str() != Some("user") {
                continue;
            }

            let mut texts: Vec<String> = Vec::new();
            if let Some(content) = event["message"]["content"].as_str() {
                texts.push(strip_ide_preamble(content));
            } else if let Some(blocks) = event["message"]["content"].as_array() {
                for b in blocks {
                    if b["type"].as_str() == Some("text") {
                        texts.push(strip_ide_preamble(b["text"].as_str().unwrap_or("")));
                    }
                }
            }

            let mut found: Option<String> = None;
            for text in &texts {
                // pos/needle.len() are byte offsets into text.to_lowercase(), NOT into
                // `text` itself — case-folding some characters changes their UTF-8 byte
                // length (e.g. Turkish İ, German ẞ), so a straight `text[pos..]` slice
                // using the LOWERCASED string's offsets can land mid-character in the
                // ORIGINAL string and panic (confirmed: "ẞẞxquilt" searching "quilt"
                // panics with "byte index 5 is not a char boundary"). Across the JNI
                // boundary a Rust panic is undefined behavior (unwinding into a JVM
                // frame), not a catchable Java exception — this crashed the whole
                // Eclipse process with no JVM crash dump and nothing in dmesg, exactly
                // matching a real user report. Snapping start/end to the nearest valid
                // char boundary in `text` (not truncating to the lowercased string,
                // which would need re-deriving positions entirely) keeps the fix local
                // and the snippet's casing exactly as the user typed it.
                if let Some(pos) = text.to_lowercase().find(&needle) {
                    let raw_start = pos.saturating_sub(40).min(text.len());
                    let raw_end = (pos + needle.len() + 40).min(text.len());
                    let start = floor_char_boundary(text, raw_start);
                    let end = ceil_char_boundary(text, raw_end);
                    found = Some(text[start..end].trim().to_string());
                    break;
                }
            }

            if let Some(snippet) = found {
                results.push(serde_json::json!({
                    "sessionId": session_id,
                    "snippet": snippet,
                }));
                break;   // one match is enough — move to the next session
            }
        }
    }

    serde_json::to_string(&results).unwrap_or_else(|_| "[]".into())
}

// ---------------------------------------------------------------------------
// load_session_history — read a specific session's JSONL and return the
// conversation as an ordered list of render items so the GUI can reconstruct
// EXACTLY how the live session looked:
//   {t:user, content}      - user message (raw; GUI parses the ide_selection chip)
//   {t:thinking}           - a thinking block (shown as "Thinking", no duration)
//   {t:tool, name, input}  - a tool call (Read/Edit/Search/Asking... + inline diff)
//   {t:answered, text}     - the user's answer to an askUserQuestion card
//   {t:text, text}         - assistant prose
// Each assistant item carries the model that turn ran on so the GUI can resume
// the conversation with its last-used model and show it in the status bar.
// ---------------------------------------------------------------------------

/// A subagent's (Task/Agent tool) own accumulated nested transcript — the on-disk
/// counterpart of chat.js's live `agentLogs` entries, so a reopened conversation's agents
/// keep their duration/tokens/prompt/tool-call list/"Open transcript" instead of losing it
/// the moment the webview holding the live version is torn down.
///
/// NOT reconstructed from the top-level transcript file: a subagent's own conversation is
/// NEVER multiplexed into its parent's `.jsonl` (confirmed on disk — there is no
/// parent_tool_use_id/parentToolUseId anywhere in it, live-stream-only). It gets its OWN
/// file instead, at `<projects_dir>/<session_id>/subagents/agent-<id>.jsonl`, with an
/// `agent-<id>.meta.json` sidecar whose `toolUseId` field is the top-level Agent tool_use's
/// own id — see `read_agent_log`, which finds and parses that file directly.
///
/// `items` matches chat.js's own shape exactly ({"kind":"text"|"thinking"|"tool", ...}) so
/// history.js can hand it to buildAgentLogItemEl with no translation. Unlike the live
/// version, an item's text/thinking blocks never need delta accumulation — every assistant
/// line in a SAVED transcript is already the complete, final message (`partial:true` lines
/// are skipped, same as the top-level reconstruction already does).
struct AgentLogAccum {
    items: Vec<serde_json::Value>,
    /// The subagent's OWN tool_use ids → index into `items`, so ITS tool_results (from
    /// the SAME dedicated file) can stamp the right step — a separate id space from the
    /// top-level `tool_idx` used elsewhere in this function.
    tool_idx: HashMap<String, usize>,
    tokens: u64,
    model: String,
    started_at: Option<String>,
    ended_at: Option<String>,
}

/// Finds and parses a subagent's own dedicated transcript file, given the PARENT
/// session's own projects directory/id and the Agent tool_use's own id (matched against
/// each `agent-*.meta.json`'s `toolUseId` field — see AgentLogAccum's doc comment for the
/// directory layout this assumes). Returns `None` when no matching subagent file exists
/// (an older conversation predating this feature, or a tool call that was never an
/// Agent/Task in the first place).
fn read_agent_log(projects_dir: &std::path::Path, session_id: &str, tool_use_id: &str) -> Option<AgentLogAccum> {
    let subagents_dir = projects_dir.join(session_id).join("subagents");
    let mut jsonl_path: Option<std::path::PathBuf> = None;
    for entry in fs::read_dir(&subagents_dir).ok()?.flatten() {
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if !name.ends_with(".meta.json") {
            continue;
        }
        // A malformed sidecar only rules out its own agent, not the rest of the folder.
        let Some(meta) = fs::read_to_string(&path).ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok()) else { continue; };
        if meta["toolUseId"].as_str() == Some(tool_use_id) {
            let jsonl_name = format!("{}.jsonl", name.trim_end_matches(".meta.json"));
            jsonl_path = Some(subagents_dir.join(jsonl_name));
            break;
        }
    }
    let file = fs::File::open(jsonl_path?).ok()?;
    let mut accum = AgentLogAccum {
        items: Vec::new(), tool_idx: HashMap::new(), tokens: 0, model: String::new(),
        started_at: None, ended_at: None,
    };
    for line in BufReader::new(file).lines() {
        let line = match line {
            Ok(l) if !l.is_empty() => l,
            _ => continue,
        };
        let event: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(ts) = event["timestamp"].as_str() {
            if !ts.is_empty() {
                if accum.started_at.is_none() {
                    accum.started_at = Some(ts.to_string());
                }
                accum.ended_at = Some(ts.to_string());
            }
        }
        match event["type"].as_str() {
            Some("assistant") => {
                // Every line in a SAVED transcript is already the final message — unlike
                // the live stream there is no partial/delta form to skip, but a defensive
                // check costs nothing if one ever did slip through.
                if event.get("partial").and_then(|v| v.as_bool()).unwrap_or(false) {
                    continue;
                }
                if let Some(m) = event["message"]["model"].as_str() {
                    if !m.is_empty() {
                        accum.model = m.to_string();
                    }
                }
                let usage = &event["message"]["usage"];
                accum.tokens += usage["input_tokens"].as_u64().unwrap_or(0)
                    + usage["output_tokens"].as_u64().unwrap_or(0)
                    + usage["cache_creation_input_tokens"].as_u64().unwrap_or(0)
                    + usage["cache_read_input_tokens"].as_u64().unwrap_or(0);
                if let Some(content) = event["message"]["content"].as_array() {
                    for b in content {
                        match b["type"].as_str() {
                            Some("text") => {
                                if let Some(t) = b["text"].as_str() {
                                    if !t.is_empty() {
                                        accum.items.push(serde_json::json!({ "kind": "text", "text": t }));
                                    }
                                }
                            }
                            Some("thinking") => {
                                let t = b["thinking"].as_str().unwrap_or("");
                                if !t.is_empty() {
                                    accum.items.push(serde_json::json!({ "kind": "thinking", "text": t }));
                                }
                            }
                            Some("tool_use") => {
                                let name = b["name"].as_str().unwrap_or("tool");
                                let input = if b["input"].is_null() {
                                    serde_json::json!({})
                                } else {
                                    b["input"].clone()
                                };
                                accum.items.push(serde_json::json!({ "kind": "tool", "name": name, "input": input }));
                                if let Some(id) = b["id"].as_str() {
                                    accum.tool_idx.insert(id.to_string(), accum.items.len() - 1);
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            Some("user") => {
                if let Some(blocks) = event["message"]["content"].as_array() {
                    for b in blocks {
                        if b["type"].as_str() != Some("tool_result") {
                            continue;
                        }
                        let tuid = b["tool_use_id"].as_str().unwrap_or("");
                        if tuid.is_empty() {
                            continue;
                        }
                        let is_err = b["is_error"].as_bool().unwrap_or(false);
                        let text = if is_err {
                            tool_error_summary(&flatten_result_content(b)).unwrap_or_default()
                        } else {
                            flatten_result_content(b)
                        };
                        if let Some(&idx) = accum.tool_idx.get(tuid) {
                            if let Some(obj) = accum.items.get_mut(idx).and_then(|v| v.as_object_mut()) {
                                obj.insert("status".into(),
                                    serde_json::Value::from(if is_err { "interrupted" } else { "done" }));
                                if is_err {
                                    obj.insert("errorText".into(), serde_json::Value::from(text.as_str()));
                                } else {
                                    obj.insert("resultText".into(), serde_json::Value::from(text.as_str()));
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Some(accum)
}

pub fn load_session_history(workspace_root: &str, session_id: &str) -> String {
    let dir = match projects_dir(workspace_root) {
        Some(d) => d,
        None => return "[]".into(),
    };
    if session_id.is_empty() {
        return "[]".into();
    }

    let path = dir.join(format!("{}.jsonl", session_id));
    let file = match fs::File::open(&path) {
        Ok(f) => f,
        Err(_) => return "[]".into(),
    };
    let reader = BufReader::new(file);

    let mut items: Vec<serde_json::Value> = Vec::new();
    // tool_use ids of askUserQuestion calls, so their answers can be surfaced.
    let mut ask_ids: HashSet<String> = HashSet::new();
    // Map each tool_use id → the index of its item in `items`, so a later
    // tool_result can stamp that tool's outcome (finished vs. interrupted).
    let mut tool_idx: HashMap<String, usize> = HashMap::new();
    // tool_use id → whether its tool_result reported an error (interrupt/reject).
    let mut result_error: HashMap<String, bool> = HashMap::new();
    // tool_use id → the one-line reason a failed tool gave, for the muted line
    // under its tool row. Only failures the user did not cause are recorded —
    // see `tool_error_summary`, which returns None for their own decisions.
    let mut result_text: HashMap<String, String> = HashMap::new();
    // tool_use id → the FULL result text for a SUCCESSFUL tool — separate map from
    // result_text above, which is error-only and pre-condensed to one line. This one
    // feeds the same "OUT" rendering (chat.js's renderToolOutput) the live path uses via
    // chat.rs's build_status_json-adjacent tool_result handler — without it, a reloaded/
    // resumed conversation showed tool input but never its output, since history.js's
    // reconstruction never had anything but errorText to hand makeToolLine.
    let mut result_success_text: HashMap<String, String> = HashMap::new();

    for line in reader.lines() {
        let line = match line {
            Ok(l) if !l.is_empty() => l,
            _ => continue,
        };
        let event: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        match event["type"].as_str() {
            Some("user") => {
                let content = &event["message"]["content"];
                if let Some(c) = content.as_str() {
                    // A post-compaction summary is stored as a user line flagged
                    // isCompactSummary — surface it as the expandable "Compacted
                    // chat" body, never as a (huge) user bubble.
                    if is_non_user_notice(&event, c) || is_task_notification(&event, c) {
                        // Not something the user typed (the browser-disconnected notice,
                        // or a background-task notification): Claude's reply to it is what
                        // the conversation shows.
                    } else if event["isCompactSummary"].as_bool().unwrap_or(false) {
                        items.push(serde_json::json!({ "t": "compact_summary", "text": c }));
                    } else {
                        let mut item = serde_json::json!({ "t": "user", "content": c });
                        // The transcript uuid, so the GUI can target THIS message for
                        // per-message actions (rewind/fork/delete). Only set when the
                        // line actually carries one — an id-less item isn't targetable.
                        if let Some(u) = event["uuid"].as_str() {
                            if !u.is_empty() {
                                item["id"] = serde_json::Value::from(u);
                            }
                        }
                        // ISO 8601, same field list_sessions already reads for its own
                        // sort key — forwarded so the GUI can show it above the bubble
                        // (opt-in preference), not currently used for anything else here.
                        if let Some(ts) = event["timestamp"].as_str() {
                            if !ts.is_empty() {
                                item["ts"] = serde_json::Value::from(ts);
                            }
                        }
                        items.push(item);
                    }
                } else if let Some(blocks) = content.as_array() {
                    // A message the user sent with pasted images is stored as
                    // content BLOCKS (text + image), not a plain string — rebuild
                    // it as one user item so the bubble and its image chips come
                    // back on reload. Images carry their base64 so the chip can
                    // draw its thumbnail; tool_result-only lines add nothing.
                    let mut text = String::new();
                    let mut images: Vec<serde_json::Value> = Vec::new();
                    let mut documents: Vec<serde_json::Value> = Vec::new();
                    // Which document block of THIS message each chip came from, so a
                    // click can find it again in the transcript.
                    let mut doc_index = 0usize;
                    for b in blocks {
                        match b["type"].as_str() {
                            Some("text") => {
                                let s = b["text"].as_str().unwrap_or("");
                                // A file uploaded as a path is one of these blocks: it
                                // comes back as its chip, not as words in the bubble.
                                if let Some(path) = attached_file_path(s) {
                                    documents.push(serde_json::json!({
                                        "title": path.rsplit(['/', '\\']).next().unwrap_or(path),
                                        "encoding": "path",
                                        "path": path,
                                    }));
                                    continue;
                                }
                                // The browser blocks a `@browser` message carries are
                                // context for the model, not words the user typed.
                                if !s.is_empty() && !is_browser_context(s) {
                                    if !text.is_empty() {
                                        text.push('\n');
                                    }
                                    text.push_str(s);
                                }
                            }
                            Some("image") => {
                                let src = &b["source"];
                                let data = src["data"].as_str().unwrap_or("");
                                if data.is_empty() {
                                    continue;
                                }
                                let mt = src["media_type"].as_str().unwrap_or("image/png");
                                images.push(
                                    serde_json::json!({ "media_type": mt, "data": data }),
                                );
                            }
                            // An uploaded file: only its name and where to find it
                            // again. The contents stay here — a transcript holds them
                            // in full, and pushing 30MB of base64 through JNI into the
                            // webview to redraw a chip would freeze the view. Clicking
                            // the chip asks for the file itself (session_document_file).
                            Some("document") => {
                                let src = &b["source"];
                                if src["data"].as_str().unwrap_or("").is_empty() {
                                    continue;
                                }
                                documents.push(serde_json::json!({
                                    "title": b["title"].as_str().unwrap_or(""),
                                    "media_type": src["media_type"].as_str().unwrap_or(""),
                                    "encoding": src["type"].as_str().unwrap_or(""),
                                    "index": doc_index,
                                }));
                                doc_index += 1;
                            }
                            _ => {}
                        }
                    }
                    if !text.is_empty() || !images.is_empty() || !documents.is_empty() {
                        let mut item = serde_json::json!({ "t": "user", "content": text });
                        if !images.is_empty() {
                            item["images"] = serde_json::Value::Array(images);
                        }
                        if !documents.is_empty() {
                            item["documents"] = serde_json::Value::Array(documents);
                        }
                        if let Some(u) = event["uuid"].as_str() {
                            if !u.is_empty() {
                                item["id"] = serde_json::Value::from(u);
                            }
                        }
                        if let Some(ts) = event["timestamp"].as_str() {
                            if !ts.is_empty() {
                                item["ts"] = serde_json::Value::from(ts);
                            }
                        }
                        items.push(item);
                    }
                    for b in blocks {
                        if b["type"].as_str() != Some("tool_result") {
                            continue;
                        }
                        let tuid = b["tool_use_id"].as_str().unwrap_or("");
                        // Record the tool's outcome so its dot can be reconstructed:
                        // is_error ⇒ interrupted/rejected, otherwise finished. (A tool
                        // with no result at all stays unresolved → interrupted below.)
                        if !tuid.is_empty() {
                            let is_err = b["is_error"].as_bool().unwrap_or(false);
                            result_error.insert(tuid.to_string(), is_err);
                            // Keep WHY it failed, not just that it did — reloading a
                            // conversation used to leave a bare red dot with the reason
                            // thrown away, so a past failure read as an unexplained stop.
                            if is_err {
                                if let Some(sum) = tool_error_summary(&flatten_result_content(b)) {
                                    result_text.insert(tuid.to_string(), sum);
                                }
                            } else {
                                // Full text, no condensing — chat.js caps/links-out to a
                                // full view for long content on its own (capIfOverflowing),
                                // same as the live path.
                                let full = flatten_result_content(b);
                                if !full.is_empty() {
                                    result_success_text.insert(tuid.to_string(), full);
                                }
                            }
                        }
                        if !ask_ids.contains(tuid) {
                            continue;
                        }
                        let rc = strip_answer_prefix(&flatten_result_content(b));
                        if !rc.is_empty() {
                            items.push(serde_json::json!({ "t": "answered", "text": rc }));
                        }
                    }
                }
            }
            Some("assistant") => {
                // Only include non-partial (final) assistant messages.
                if event.get("partial").and_then(|v| v.as_bool()).unwrap_or(false) {
                    continue;
                }
                let content = match event["message"]["content"].as_array() {
                    Some(c) => c,
                    None => continue,
                };
                // A synthetic assistant message standing in for a backend error
                // (529 overload, session-limit hit, …). The CLI flags it
                // isApiErrorMessage — verified on disk for both of those texts —
                // and live it renders as the muted "⚠ …" line via onError, never
                // as a paragraph. Reload has to rebuild that same muted line, so
                // surface it as its own item type rather than ordinary text.
                if event["isApiErrorMessage"].as_bool().unwrap_or(false) {
                    let mut text = String::new();
                    for b in content {
                        if b["type"].as_str() != Some("text") {
                            continue;
                        }
                        let s = b["text"].as_str().unwrap_or("");
                        if !s.is_empty() {
                            if !text.is_empty() {
                                text.push('\n');
                            }
                            text.push_str(s);
                        }
                    }
                    if !text.is_empty() {
                        items.push(serde_json::json!({ "t": "error", "text": text }));
                    }
                    continue;
                }
                // The model this turn ran on — attached to each item.
                let model = event["message"]["model"].as_str().unwrap_or("");
                for b in content {
                    match b["type"].as_str() {
                        Some("thinking") => {
                            let tt = b["thinking"].as_str().unwrap_or("");
                            items.push(serde_json::json!({
                                "t": "thinking", "model": model, "text": tt,
                            }));
                        }
                        Some("text") => {
                            if let Some(t) = b["text"].as_str() {
                                if !t.is_empty() {
                                    items.push(serde_json::json!({
                                        "t": "text", "text": t, "model": model,
                                    }));
                                }
                            }
                        }
                        Some("tool_use") => {
                            let name = b["name"].as_str().unwrap_or("tool");
                            let input = if b["input"].is_null() {
                                serde_json::json!({})
                            } else {
                                b["input"].clone()
                            };
                            let id = b["id"].as_str().unwrap_or("");
                            items.push(serde_json::json!({
                                // Its own tool_use id — needed so history.js can set
                                // data-tuid the same way the live path does (addToolLine),
                                // which is what an Agent/Task line's own reconstructed
                                // agentLog (below) gets matched up against.
                                "t": "tool", "name": name, "input": input, "model": model, "id": id,
                            }));
                            if !id.is_empty() {
                                // Remember where this tool sits so its result can stamp
                                // a status onto it after the whole file is read.
                                tool_idx.insert(id.to_string(), items.len() - 1);
                                if name.to_ascii_lowercase().contains("askuserquestion") {
                                    ask_ids.insert(id.to_string());
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            Some("system") => {
                // Compaction marker (written by /compact or auto-compact). The
                // jsonl uses camelCase compactMetadata (unlike the stream's
                // compact_metadata) — verified against CLI 2.1.177.
                if event["subtype"].as_str() == Some("compact_boundary") {
                    let md = &event["compactMetadata"];
                    items.push(serde_json::json!({
                        "t": "compact",
                        "trigger": md["trigger"].as_str().unwrap_or("manual"),
                        "preTokens": md["preTokens"].as_u64().unwrap_or(0),
                        "postTokens": md["postTokens"].as_u64().unwrap_or(0),
                    }));
                }
                // Where a conversation pulled down from claude.ai ends and the
                // local one continues. Written by teleport::run into the
                // transcript, so it survives into history like any other event.
                if event["subtype"].as_str() == Some("teleported_from_web") {
                    items.push(serde_json::json!({ "t": "teleported" }));
                }
            }
            _ => {}
        }
    }

    // Stamp each tool with a reconstructed dot status so reloading a past
    // conversation keeps the green/red it had live:
    //   • result present, not an error → "done"        (finished, green)
    //   • result present with is_error → "interrupted"  (rejected/stopped, red)
    //   • no result at all             → "interrupted"  (turn was cut off, red)
    for (id, &idx) in &tool_idx {
        let status = match result_error.get(id) {
            Some(false) => "done",
            Some(true) => "interrupted",
            None => "interrupted",
        };
        if let Some(obj) = items.get_mut(idx).and_then(|v| v.as_object_mut()) {
            obj.insert("status".into(), serde_json::Value::from(status));
            // The reason, when the failure was the tool's own. A cut-off turn has
            // no result and so no text — the red dot alone still says "stopped".
            if let Some(txt) = result_text.get(id) {
                obj.insert("errorText".into(), serde_json::Value::from(txt.as_str()));
            }
            // The successful tool's actual output — makeToolLine (chat.js) renders this
            // into an OUT box/result-list/checklist exactly like the live path does.
            if let Some(txt) = result_success_text.get(id) {
                obj.insert("resultText".into(), serde_json::Value::from(txt.as_str()));
            }
        }
    }

    // Attach each Agent/Task tool's own nested log onto its top-level item — read
    // straight from its own dedicated file (see read_agent_log's doc comment: a
    // subagent's conversation is never multiplexed into its PARENT's own transcript at
    // all, confirmed on disk). history.js hands this to chat.js's ensureAgentLog/
    // agentLogs so a reopened conversation's /agents popup (duration, tokens, model,
    // Prompt, Tool calls, "Open transcript") works the same as it did live.
    for (id, &idx) in &tool_idx {
        let is_agent = items.get(idx)
            .and_then(|v| v["name"].as_str())
            .map(|n| { let n = n.to_ascii_lowercase(); n == "agent" || n == "task" })
            .unwrap_or(false);
        if !is_agent {
            continue;
        }
        if let Some(accum) = read_agent_log(&dir, session_id, id) {
            if let Some(obj) = items.get_mut(idx).and_then(|v| v.as_object_mut()) {
                obj.insert("agentLog".into(), serde_json::json!({
                    "items": accum.items,
                    "tokens": accum.tokens,
                    "model": accum.model,
                    "startedAt": accum.started_at,
                    "endedAt": accum.ended_at,
                }));
            }
        }
    }

    serde_json::to_string(&items).unwrap_or_else(|_| "[]".into())
}

/// Flattens a `tool_result` block's content to plain text. The CLI writes it
/// either as a bare string or as `[{type:"text",…}]` blocks, so both shapes have
/// to collapse to the same thing.
pub(crate) fn flatten_result_content(b: &serde_json::Value) -> String {
    let mut out = String::new();
    if let Some(s) = b["content"].as_str() {
        out.push_str(s);
    } else if let Some(parts) = b["content"].as_array() {
        for rb in parts {
            if rb["type"].as_str() == Some("text") {
                out.push_str(rb["text"].as_str().unwrap_or(""));
            }
        }
    }
    out
}

/// Writes one uploaded document out of a transcript to a file and returns its
/// path, or `""` when it isn't there. `index` counts document blocks within the
/// message `message_uuid`, as `load_session_history` numbered them.
///
/// The bytes never go to Java: a transcript keeps every uploaded file in full, so
/// the page gets a name and this locator, and only a click on the chip spends the
/// copy — of that one file, straight to disk for the OS to open.
pub fn session_document_file(
    workspace_root: &str,
    session_id: &str,
    message_uuid: &str,
    index: usize,
) -> String {
    let Some(dir) = projects_dir(workspace_root) else { return String::new() };
    if session_id.is_empty() || message_uuid.is_empty() {
        return String::new();
    }
    let Ok(file) = fs::File::open(dir.join(format!("{session_id}.jsonl"))) else {
        return String::new();
    };
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let Ok(event) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
        if event["uuid"].as_str() != Some(message_uuid) {
            continue;
        }
        let Some(blocks) = event["message"]["content"].as_array() else { continue };
        let mut seen = 0usize;
        for b in blocks {
            if b["type"].as_str() != Some("document")
                || b["source"]["data"].as_str().unwrap_or("").is_empty()
            {
                continue;
            }
            if seen == index {
                return write_document_file(b);
            }
            seen += 1;
        }
    }
    String::new()
}

/// One document block's contents, written under the temp directory and named after
/// the file it came from. Returns the path, or `""` if anything failed.
fn write_document_file(block: &serde_json::Value) -> String {
    use base64::Engine as _;
    let src = &block["source"];
    let data = src["data"].as_str().unwrap_or("");
    let bytes = if src["type"].as_str() == Some("base64") {
        match base64::engine::general_purpose::STANDARD.decode(data) {
            Ok(b) => b,
            Err(_) => return String::new(),
        }
    } else {
        data.as_bytes().to_vec()
    };
    let title = block["title"].as_str().unwrap_or("");
    let safe: String = title
        .chars()
        .map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c })
        .collect();
    let name = if safe.trim().is_empty() { "attachment".to_string() } else { safe };
    let dir = std::env::temp_dir().join(format!("claude-attachment-{}", std::process::id()));
    if fs::create_dir_all(&dir).is_err() {
        return String::new();
    }
    let out = dir.join(name);
    if fs::write(&out, bytes).is_err() {
        return String::new();
    }
    out.to_string_lossy().into_owned()
}

/// The `<browser_instruction>` and `<browser tabGroupId=…>` text blocks sent
/// alongside a message that mentions the browser.
fn is_browser_context(text: &str) -> bool {
    text.starts_with("<browser_instruction>") || text.starts_with("<browser tabGroupId=\"")
}

/// The path in an `<attached_file path="…" />` block — the whole block and nothing
/// else, so a message that merely quotes one stays the user's own words.
fn attached_file_path(text: &str) -> Option<&str> {
    let rest = text.strip_prefix("<attached_file path=\"")?;
    let (path, tail) = rest.split_once('"')?;
    (tail == " />" && !path.is_empty()).then_some(path)
}

/// Longest error summary we surface. The full text stays in the transcript; the
/// GUI shows one line, and real results run to 100+ lines.
const ERROR_SUMMARY_MAX: usize = 160;

/// Prefixes that mark a result as the USER'S OWN decision rather than a tool
/// failure. The CLI reports "declined", "rejected" and "answered instead" through
/// the same `is_error` channel a genuine failure uses, but the GUI already shows
/// those through its decision cards — repeating the sentence under the tool row
/// would be noise. Verified against 111 real `is_error` results: 23 are these.
const DECISION_PREFIXES: [&str; 4] = [
    "The user doesn't want to proceed",
    "The user declined",
    "The user dismissed",
    "[User typed]:",
];

/// Condenses a failed tool's result into the single muted line shown beneath it,
/// or `None` when nothing should be shown.
///
/// Returns `None` for the user's own decisions (see [`DECISION_PREFIXES`]) so a
/// declined tool keeps its red dot and stays quiet.
///
/// A bare `Exit code N` first line is joined to the next real line: three
/// quarters of genuine failures lead with it, and the number alone says nothing
/// about what broke. The exit status is kept rather than dropped because 143
/// (timeout) and 1 (ordinary failure) mean different things.
pub(crate) fn tool_error_summary(raw: &str) -> Option<String> {
    let mut t = raw.trim();
    if t.is_empty() {
        return None;
    }
    if DECISION_PREFIXES.iter().any(|p| t.starts_with(p)) {
        return None;
    }
    // Unwrap the CLI's own error envelope so the message reads plainly.
    if let Some(inner) = t.strip_prefix("<tool_use_error>") {
        t = inner.strip_suffix("</tool_use_error>").unwrap_or(inner).trim();
    }
    let mut lines = t.lines().map(str::trim).filter(|l| !l.is_empty());
    let head = lines.next()?;
    let mut summary = head.to_string();
    if is_bare_exit_code(head) {
        if let Some(next) = lines.next() {
            summary.push_str(" · ");
            summary.push_str(next);
        }
    }
    if summary.is_empty() {
        return None;
    }
    // char_indices, not byte slicing — these carry paths and prose that are not
    // guaranteed ASCII, and a mid-codepoint cut would panic.
    if summary.chars().count() > ERROR_SUMMARY_MAX {
        let cut: String = summary.chars().take(ERROR_SUMMARY_MAX).collect();
        summary = format!("{}…", cut.trim_end());
    }
    Some(summary)
}

/// True for a line that is exactly "Exit code <digits>" and nothing else.
fn is_bare_exit_code(line: &str) -> bool {
    match line.strip_prefix("Exit code ") {
        Some(rest) => !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()),
        None => false,
    }
}

/// Drops a leading "The user answered: " (any case, any leading whitespace)
/// from an askUserQuestion tool_result, leaving just the chosen answer.
fn strip_answer_prefix(s: &str) -> String {
    const PREFIX: &str = "the user answered:";
    let t = s.trim_start();
    let matched = t
        .get(..PREFIX.len())
        .map_or(false, |p| p.eq_ignore_ascii_case(PREFIX));
    if matched {
        t[PREFIX.len()..].trim_start().to_string()
    } else {
        s.to_string()
    }
}

// ---------------------------------------------------------------------------
// message_ids / delete_message — per-message actions inside one transcript
//
// The jsonl is a parentUuid-linked chain, so dropping a line means re-linking
// its children onto that line's OWN parent; a plain filter leaves a dangling
// reference in the chain the CLI walks on `--resume`.
//
// The raw prompt is also stored OUTSIDE the chain, in line types that carry no
// uuid at all: `queue-operation.content` (what was typed, `<ide_context …>`
// wrapper included) and `last-prompt.lastPrompt` (rewritten every time the leaf
// advances, so one message leaves many copies). Measured on a live transcript:
// a single message existed 9 times — 1 chained, 6 last-prompt, 2
// queue-operation. Removing only the chained line leaves the text on disk while
// every UI surface reports success (the readers here and in the CLI both render
// line-by-line and never look at these types), so the copies are stripped too
// and the result is asserted BEFORE anything is written.
//
// `file-history-snapshot` lines are deliberately left untouched: RewindService
// forward-merges them in first-appearance order to recover pre-first-edit
// backups, so dropping one silently corrupts rewinding to EARLIER messages.
// ---------------------------------------------------------------------------

/// The user messages the GUI draws as bubbles, in order, as
/// `[{"id":<uuid>,"text":<raw content>}]`. Derived from `load_session_history`'s
/// own output, so these can never drift from the rendered items.
///
/// The text ships with the id because position alone cannot identify a bubble: a
/// message queued mid-stream is on screen BEFORE its transcript line exists, so
/// the two sequences differ in length and pairing by index (from either end)
/// mis-assigns. The caller matches on text instead.
pub fn message_ids(workspace_root: &str, session_id: &str) -> String {
    let items: Vec<serde_json::Value> =
        serde_json::from_str(&load_session_history(workspace_root, session_id))
            .unwrap_or_default();
    let out: Vec<serde_json::Value> = items
        .iter()
        .filter(|it| it["t"].as_str() == Some("user"))
        .filter_map(|it| {
            let id = it["id"].as_str()?;
            Some(serde_json::json!({ "id": id, "text": it["content"].as_str().unwrap_or("") }))
        })
        .collect();
    serde_json::to_string(&out).unwrap_or_else(|_| "[]".into())
}

/// The typed prompt a line carries, or None when it isn't a real user message
/// (tool_result turns and every non-user line included).
fn prompt_text(event: &serde_json::Value) -> Option<String> {
    if event["type"].as_str() != Some("user") {
        return None;
    }
    let content = &event["message"]["content"];
    if let Some(s) = content.as_str() {
        return Some(s.to_string());
    }
    let mut text = String::new();
    for b in content.as_array()? {
        if b["type"].as_str() != Some("text") {
            continue;
        }
        if let Some(s) = b["text"].as_str() {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(s);
        }
    }
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

/// Whether a bookkeeping field holds the prompt being removed. The queue log
/// keeps the text with its `<ide_context …>` wrapper still attached, so an exact
/// match would miss it — compare stripped forms, either containing the other.
fn same_prompt(field: &str, target: &str) -> bool {
    let a = strip_ide_preamble(field).trim().to_string();
    let b = strip_ide_preamble(target).trim().to_string();
    if a.is_empty() || b.is_empty() {
        return false;
    }
    a == b || a.contains(&b) || b.contains(&a)
}

/// Any string anywhere in the line still holding `needle`. Walks the parsed
/// value rather than the raw text so JSON escaping can't hide a match.
fn holds_prompt(v: &serde_json::Value, needle: &str) -> bool {
    match v {
        serde_json::Value::String(s) => strip_ide_preamble(s).trim().contains(needle),
        serde_json::Value::Array(a) => a.iter().any(|x| holds_prompt(x, needle)),
        serde_json::Value::Object(o) => o.values().any(|x| holds_prompt(x, needle)),
        _ => false,
    }
}

/// Permanently removes one user message from a session transcript.
/// Returns `{"ok":true,"stripped":N}` or `{"error":"…"}` — N being the unchained
/// bookkeeping copies cleared alongside the message itself.
pub fn delete_message(workspace_root: &str, session_id: &str, message_id: &str) -> String {
    match delete_message_inner(workspace_root, session_id, message_id) {
        Ok(n) => serde_json::json!({ "ok": true, "stripped": n }).to_string(),
        Err(e) => serde_json::json!({ "error": e }).to_string(),
    }
}

fn delete_message_inner(
    workspace_root: &str,
    session_id: &str,
    message_id: &str,
) -> Result<usize, String> {
    if session_id.is_empty()
        || session_id.contains('/')
        || session_id.contains('\\')
        || session_id.contains("..")
    {
        return Err("Bad session id.".into());
    }
    if message_id.is_empty() {
        return Err("Bad message id.".into());
    }
    let dir = projects_dir(workspace_root).ok_or("No transcripts for this workspace.")?;
    let path = dir.join(format!("{}.jsonl", session_id));
    let raw =
        fs::read_to_string(&path).map_err(|e| format!("Cannot read the transcript ({e})."))?;
    let eol = if raw.contains("\r\n") { "\r\n" } else { "\n" };
    let lines: Vec<&str> = raw.lines().collect();
    let parsed: Vec<Option<serde_json::Value>> = lines
        .iter()
        .map(|l| {
            if l.trim().is_empty() {
                None
            } else {
                serde_json::from_str(l).ok()
            }
        })
        .collect();

    let target = parsed
        .iter()
        .position(|e| {
            e.as_ref().map_or(false, |e| {
                e["type"].as_str() == Some("user") && e["uuid"].as_str() == Some(message_id)
            })
        })
        .ok_or("That message is no longer in this conversation.")?;
    let text = parsed[target].as_ref().and_then(prompt_text).unwrap_or_default();
    let dead_parent = parsed[target]
        .as_ref()
        .map(|e| e["parentUuid"].clone())
        .unwrap_or(serde_json::Value::Null);

    // The span this message owns: from the previous typed prompt to the next one.
    // Its bookkeeping copies live inside that window (queue-operation just ahead
    // of the message, last-prompt repeatedly after it). The span does NOT by
    // itself separate this message's copies from the previous message's trailing
    // ones — those sit inside it too — that is what `same_prompt` is for; the span
    // keeps the sweep and the assertion off messages further away. Two CONSECUTIVE
    // prompts with identical text can therefore clear each other's bookkeeping
    // field, which is harmless (the other message's own line is untouched).
    let is_boundary = |i: usize| {
        parsed[i]
            .as_ref()
            .map_or(false, |e| prompt_text(e).is_some())
    };
    let start = (0..target).rev().find(|&i| is_boundary(i)).map_or(0, |i| i + 1);
    let end = ((target + 1)..parsed.len())
        .find(|&i| is_boundary(i))
        .unwrap_or(parsed.len());

    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut span_check: Vec<serde_json::Value> = Vec::new();
    let mut stripped = 0usize;
    for (i, line) in lines.iter().enumerate() {
        if i == target {
            continue; // the message itself
        }
        let Some(orig) = parsed[i].as_ref() else {
            out.push((*line).to_string());
            continue;
        };
        let mut ev = orig.clone();
        let mut changed = false;
        let mut ty = String::new();
        if let Some(obj) = ev.as_object_mut() {
            ty = obj
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            // Children of the removed line adopt its parent, so the chain still
            // closes for `--resume`.
            for key in ["parentUuid", "logicalParentUuid", "leafUuid"] {
                if obj.get(key).and_then(|v| v.as_str()) == Some(message_id) {
                    obj.insert(key.to_string(), dead_parent.clone());
                    changed = true;
                }
            }
            let field = match ty.as_str() {
                "queue-operation" => Some("content"),
                "last-prompt" => Some("lastPrompt"),
                _ => None,
            };
            if let Some(field) = field {
                let hit = i >= start
                    && i < end
                    && obj
                        .get(field)
                        .and_then(|v| v.as_str())
                        .map_or(false, |s| same_prompt(s, &text));
                if hit {
                    obj.remove(field);
                    changed = true;
                    stripped += 1;
                }
            }
        }
        // Assertion set: everything in this message's span that ISN'T a message
        // in its own right. `assistant` lines are skipped because Claude quoting
        // the text back is legitimate; `user` lines are skipped because they are
        // other people's messages. Anything else — including a line type a
        // future CLI adds — must come out clean.
        if i >= start && i < end && ty != "user" && ty != "assistant" {
            span_check.push(ev.clone());
        }
        out.push(if changed {
            ev.to_string()
        } else {
            (*line).to_string()
        });
    }

    let needle = strip_ide_preamble(&text).trim().to_string();
    if !needle.is_empty() {
        if let Some(bad) = span_check.iter().find(|e| holds_prompt(e, &needle)) {
            return Err(format!(
                "The message text is still present in a \"{}\" line — the transcript was left untouched.",
                bad["type"].as_str().unwrap_or("transcript")
            ));
        }
    }

    // Replace via a sibling temp file so a crash mid-write can't truncate the
    // transcript.
    let tmp = dir.join(format!("{}.jsonl.tmp", session_id));
    {
        let mut f =
            fs::File::create(&tmp).map_err(|e| format!("Cannot write the transcript ({e})."))?;
        for l in &out {
            f.write_all(l.as_bytes())
                .and_then(|_| f.write_all(eol.as_bytes()))
                .map_err(|e| format!("Cannot write the transcript ({e})."))?;
        }
    }
    fs::rename(&tmp, &path).map_err(|e| format!("Cannot replace the transcript ({e})."))?;
    Ok(stripped)
}

// ---------------------------------------------------------------------------
// delete_session — remove one local session file
// ---------------------------------------------------------------------------

/// Deletes `~/.claude/projects/<hash>/<sessionId>.jsonl`. The id is rejected if
/// it could escape the projects directory (path separators or "..").
pub fn delete_session(workspace_root: &str, session_id: &str) -> bool {
    if session_id.is_empty()
        || session_id.contains('/')
        || session_id.contains('\\')
        || session_id.contains("..")
    {
        return false;
    }
    let dir = match projects_dir(workspace_root) {
        Some(d) => d,
        None => return false,
    };
    let path = dir.join(format!("{}.jsonl", session_id));
    path.is_file() && fs::remove_file(&path).is_ok()
}

// ---------------------------------------------------------------------------
// rename_session_offline — rename a session that has NO live process, by
// resuming it headless and sending the CLI's rename_session control request.
// ---------------------------------------------------------------------------

/// Renames an inactive session the CLI-native way (verified on claude 2.1.177):
/// spawn `claude -p --resume <id> --input-format stream-json --output-format
/// stream-json --verbose`, write one `rename_session` control request, close
/// stdin. The CLI appends a `custom-title` event to the ORIGINAL session jsonl
/// (no fork), runs zero model turns (zero cost) and exits on its own. Success =
/// the CLI's control_response for our request id. Blocks up to ~15s; callers
/// run it off the UI thread.
pub fn rename_session_offline(
    claude_cmd: &str,
    workspace_root: &str,
    session_id: &str,
    title: &str,
) -> bool {
    if claude_cmd.is_empty() || session_id.is_empty() || title.is_empty() {
        return false;
    }

    // crate::launch handles Windows PATH/PATHEXT resolution (bare `claude`
    // → `claude.cmd`) and `.cmd` shim quoting, same as the chat spawn paths.
    let args: Vec<String> = [
        "-p",
        "--resume",
        session_id,
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let mut cmd = crate::launch::claude_command(claude_cmd, &args);
    cmd.current_dir(workspace_root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    #[cfg(windows)]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW

    for (k, v) in crate::shell_env::captured_env().to_inject() {
        cmd.env(k, v);
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(_) => return false,
    };

    let req_id = "eclipse-ren-offline";
    let request = serde_json::json!({
        "type": "control_request",
        "request_id": req_id,
        "request": { "subtype": "rename_session", "title": title }
    });

    // Write the request, then drop stdin (EOF) so the CLI exits after answering.
    if let Some(mut stdin) = child.stdin.take() {
        let ok = stdin
            .write_all(request.to_string().as_bytes())
            .and_then(|_| stdin.write_all(b"\n"))
            .and_then(|_| stdin.flush())
            .is_ok();
        drop(stdin);
        if !ok {
            crate::launch::kill_process_tree(&mut child);
            return false;
        }
    } else {
        crate::launch::kill_process_tree(&mut child);
        return false;
    }

    // Watch stdout for the success control_response. The read loop alone can
    // block forever on a silent child (auth prompt, network stall), so a
    // watchdog kills the child at the deadline — that forces stdout EOF and
    // unblocks the loop.
    let stdout = match child.stdout.take() {
        Some(s) => s,
        None => {
            crate::launch::kill_process_tree(&mut child);
            return false;
        }
    };

    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let watchdog = {
        let done = std::sync::Arc::clone(&done);
        // Killing via the child handle needs ownership; signal the watchdog to
        // kill by pid instead so the main thread keeps `child` for reaping.
        let pid = child.id();
        std::thread::spawn(move || {
            for _ in 0..150 {
                if done.load(std::sync::atomic::Ordering::Relaxed) {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            kill_pid(pid);
        })
    };

    let mut renamed = false;
    let reader = BufReader::new(stdout);
    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let event: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if event["type"].as_str() == Some("control_response")
            && event["response"]["request_id"].as_str() == Some(req_id)
            && event["response"]["subtype"].as_str() == Some("success")
        {
            renamed = true;
            break;
        }
    }
    done.store(true, std::sync::atomic::Ordering::Relaxed);

    // Reap (or put down) the child either way; the rename outcome is decided.
    crate::launch::kill_process_tree(&mut child);
    let _ = child.wait();
    let _ = watchdog.join();
    renamed
}

/// Best-effort kill by pid, used only by the offline-rename watchdog.
#[cfg(windows)]
fn kill_pid(pid: u32) {
    let _ = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .creation_flags(0x08000000) // CREATE_NO_WINDOW
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Best-effort kill by pid, used only by the offline-rename watchdog.
#[cfg(target_os = "macos")]
fn kill_pid(pid: u32) {
    let _ = Command::new("kill")
        .args(["-KILL", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Best-effort kill by pid, used only by the offline-rename watchdog.
#[cfg(target_os = "linux")]
fn kill_pid(pid: u32) {
    let _ = Command::new("kill")
        .args(["-KILL", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Best-effort kill by pid, used only by the offline-rename watchdog.
#[cfg(target_os = "freebsd")]
fn kill_pid(pid: u32) {
    let _ = Command::new("kill")
        .args(["-KILL", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

// ---------------------------------------------------------------------------
// message_text_by_uuid — the local half of the Remote Control inbound lookup
// ---------------------------------------------------------------------------

/// The text of one transcript message, found by its `uuid`.
///
/// The local counterpart to [`crate::bridge::rc_lookup_message`], and the reason
/// an inbound Remote Control message no longer depends on the network. The CLI
/// writes every turn of a conversation to `~/.claude/projects/<hash>/<id>.jsonl`
/// **including the ones that arrived over the bridge** — verified against a real
/// bridge session, where a message typed on a phone is on disk as an ordinary
/// `user` line carrying the same uuid `command_lifecycle` announced it under.
/// (It is stamped with the *host's* `entrypoint`/`promptSource`, so those two
/// fields cannot be used to tell it from a locally typed one — the uuid can.)
///
/// Why this is the primary path: the API lookup spends an OAuth credential, and
/// on macOS that credential lives in the login Keychain, where a read can be
/// refused for reasons that have nothing to do with being signed in. Every such
/// failure rendered as *silence* — the message simply never appeared. Reading
/// the file the CLI already wrote costs no round trip and no credential.
///
/// Scans backwards: an inbound message is by definition near the end.
pub(crate) fn message_text_by_uuid(
    workspace_root: &str,
    session_id: &str,
    uuid: &str,
) -> Option<String> {
    if uuid.is_empty()
        || session_id.is_empty()
        || session_id.contains('/')
        || session_id.contains('\\')
        || session_id.contains("..")
    {
        return None;
    }
    let path = projects_dir(workspace_root)?.join(format!("{}.jsonl", session_id));
    let raw = fs::read_to_string(&path).ok()?;
    for line in raw.lines().rev() {
        if line.is_empty() || !line.contains(uuid) {
            continue; // cheap reject — parsing every line of a long transcript is not free
        }
        let v: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if v["uuid"].as_str() != Some(uuid) {
            continue;
        }
        return crate::bridge::rc_incoming_text(&v);
    }
    None
}


#[cfg(test)]
mod tests;
