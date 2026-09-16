//! Claude in Chrome, for the composer's `@browser:` mentions.
//!
//! Wired the way the VS Code extension wires it. Tabs are listed by a client of
//! the CLI's own `--claude-in-chrome-mcp` stdio server, kept alive between
//! lookups, calling `tabs_context_mcp`. That tool reports only the tab groups
//! Claude has created, never the user's own tabs, so until Claude opens one the
//! list is the single "new tab" row (verified against CLI 2.1.266). Sending a
//! message that mentions the browser adds the same server to the conversation's
//! process and prepends the text blocks the extension sends.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::process::{Child, ChildStdin, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// How the `<browser_instruction>` text the extension sends begins and ends. The
/// text itself is not kept here: the user's own Claude Code CLI bundles the same
/// instruction (identical to the extension's, 4107 chars — checked against CLI
/// 2.1.266 and extension 2.1.272), so it is read from there. These anchors find
/// it and check what was read.
const INSTRUCTION_HEAD: &str = "# Claude in Chrome browser automation";
const INSTRUCTION_TAIL: &str = "call tabs_context_mcp to see what tabs are available";
/// How far past its opening backtick the literal may run; it is about 4KB.
const INSTRUCTION_WINDOW: u64 = 64 * 1024;
const SCAN_CHUNK: usize = 8 * 1024 * 1024;

/// The CLI program file as `(path, size, mtime)`, so an updated CLI is read again.
type InstructionKey = (std::path::PathBuf, u64, Option<std::time::SystemTime>);
/// What was read from that file, a miss included: a CLI the text can't be found in
/// is scanned once, not on every message that switches the browser on.
static INSTRUCTION_CACHE: Mutex<Option<(InstructionKey, Option<String>)>> = Mutex::new(None);

/// The browser instruction, read out of the installed CLI. `None` when it can't be
/// found there — a CLI that bundles it differently, or a wrapper script in place of
/// the program. Blocking: the first call reads through the CLI binary (209MB on
/// Windows, about 220ms in a release build); later calls only stat the file.
pub fn cli_instruction(claude_cmd: &str) -> Option<String> {
    let path = crate::launch::claude_program_file(claude_cmd)?;
    let meta = std::fs::metadata(&path).ok()?;
    let key = (path, meta.len(), meta.modified().ok());
    let mut cache = INSTRUCTION_CACHE.lock().unwrap_or_else(|p| p.into_inner());
    if let Some((cached, text)) = cache.as_ref() {
        if *cached == key {
            return text.clone();
        }
    }
    let text = read_instruction(&key.0);
    *cache = Some((key, text.clone()));
    text
}

/// Finds the literal in `path` — the header straight after a template's opening
/// backtick — evaluates it, and keeps it only if it reads as the whole instruction.
fn read_instruction(path: &std::path::Path) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let needle = format!("`{INSTRUCTION_HEAD}");
    let at = find_in_reader(&mut file, needle.as_bytes(), SCAN_CHUNK)?;
    file.seek(SeekFrom::Start(at)).ok()?;
    let mut window = Vec::new();
    file.take(INSTRUCTION_WINDOW).read_to_end(&mut window).ok()?;
    // Lossy: the window runs on past the literal into whatever follows it.
    let text = eval_template(&String::from_utf8_lossy(&window))?;
    let whole = text.starts_with(INSTRUCTION_HEAD)
        && text.ends_with(INSTRUCTION_TAIL)
        && text.len() > 1000
        && !text.contains('\u{FFFD}');
    whole.then_some(text)
}

/// The offset of the first `needle` in `reader`, read `chunk` bytes at a time with
/// enough carried over that a match across two reads is still found. The CLI is not
/// text throughout, so it is searched as bytes.
pub(crate) fn find_in_reader(reader: &mut impl Read, needle: &[u8], chunk: usize) -> Option<u64> {
    let carry = needle.len().checked_sub(1)?;
    let mut buf = vec![0u8; chunk.max(1) + carry];
    let (mut base, mut filled) = (0u64, 0usize);
    loop {
        let n = reader.read(&mut buf[filled..]).ok()?;
        if n == 0 {
            return None;
        }
        filled += n;
        if let Some(i) = find_bytes(&buf[..filled], needle) {
            return Some(base + i as u64);
        }
        let keep = carry.min(filled);
        buf.copy_within(filled - keep..filled, 0);
        base += (filled - keep) as u64;
        filled = keep;
    }
}

fn find_bytes(hay: &[u8], needle: &[u8]) -> Option<usize> {
    let (first, rest) = needle.split_first()?;
    let mut from = 0;
    while let Some(p) = hay[from..].iter().position(|b| b == first) {
        let at = from + p;
        if hay.len() - at < needle.len() {
            return None;
        }
        if &hay[at + 1..at + needle.len()] == rest {
            return Some(at);
        }
        from = at + 1;
    }
    None
}

type Chars<'a> = std::iter::Peekable<std::str::Chars<'a>>;

/// Evaluates the JS template literal `src` starts with: its text with JS escapes
/// decoded, where each `${…}` slot holds one quoted string literal (the CLI builds
/// the instruction's ToolSearch line that way). `None` for anything else — a
/// computed slot, a malformed escape, no closing backtick — so a wrong instruction
/// never reaches the model.
fn eval_template(src: &str) -> Option<String> {
    let mut chars = src.chars().peekable();
    if chars.next()? != '`' {
        return None;
    }
    let mut out = String::new();
    loop {
        match chars.next()? {
            '`' => return Some(out),
            '\\' => unescape(&mut chars, &mut out)?,
            '$' if chars.peek() == Some(&'{') => {
                chars.next();
                skip_space(&mut chars);
                let quote = chars.next()?;
                if quote != '"' && quote != '\'' {
                    return None;
                }
                string_literal(&mut chars, quote, &mut out)?;
                skip_space(&mut chars);
                if chars.next()? != '}' {
                    return None;
                }
            }
            // A template's line breaks read as \n whatever the source used.
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push('\n');
            }
            c => out.push(c),
        }
    }
}

fn skip_space(chars: &mut Chars<'_>) {
    while chars.peek().is_some_and(|c| c.is_whitespace()) {
        chars.next();
    }
}

/// The rest of a quoted string literal, its opening `quote` already taken.
fn string_literal(chars: &mut Chars<'_>, quote: char, out: &mut String) -> Option<()> {
    loop {
        match chars.next()? {
            c if c == quote => return Some(()),
            '\\' => unescape(chars, out)?,
            '\n' | '\r' => return None,
            c => out.push(c),
        }
    }
}

/// One JS escape sequence, its backslash already taken.
fn unescape(chars: &mut Chars<'_>, out: &mut String) -> Option<()> {
    let decoded = match chars.next()? {
        'n' => '\n',
        't' => '\t',
        'r' => '\r',
        'b' => '\u{8}',
        'f' => '\u{c}',
        'v' => '\u{b}',
        '0' if !chars.peek().is_some_and(|d| d.is_ascii_digit()) => '\0',
        '0'..='9' => return None,
        'x' => char::from_u32(hex(chars, 2)?)?,
        'u' => {
            let unit = code_point(chars)?;
            if (0xD800..0xDC00).contains(&unit) {
                // Outside the BMP, written as a surrogate pair of \u escapes.
                if chars.next()? != '\\' || chars.next()? != 'u' {
                    return None;
                }
                let low = code_point(chars)?;
                if !(0xDC00..0xE000).contains(&low) {
                    return None;
                }
                char::from_u32(0x10000 + ((unit - 0xD800) << 10) + (low - 0xDC00))?
            } else {
                char::from_u32(unit)?
            }
        }
        // A line continuation stands for nothing.
        '\n' => return Some(()),
        '\r' => {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            return Some(());
        }
        // \` \\ \$ \" \' and any other character stand for themselves.
        other => other,
    };
    out.push(decoded);
    Some(())
}

/// The digits of a `\u` escape: four hex digits, or `{…}` around one to six.
fn code_point(chars: &mut Chars<'_>) -> Option<u32> {
    if chars.peek() != Some(&'{') {
        return hex(chars, 4);
    }
    chars.next();
    let mut value: u32 = 0;
    let mut digits = 0;
    loop {
        let c = chars.next()?;
        if c == '}' {
            break;
        }
        value = value.checked_mul(16)?.checked_add(c.to_digit(16)?)?;
        digits += 1;
    }
    (digits > 0 && value <= 0x10FFFF).then_some(value)
}

fn hex(chars: &mut Chars<'_>, digits: usize) -> Option<u32> {
    (0..digits).try_fold(0u32, |value, _| Some(value * 16 + chars.next()?.to_digit(16)?))
}
const SERVER_ARG: &str = "--claude-in-chrome-mcp";
const CALL_TIMEOUT: Duration = Duration::from_secs(20);

struct Client {
    claude_cmd: String,
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
    next_id: u64,
}

impl Drop for Client {
    fn drop(&mut self) {
        crate::launch::kill_process_tree(&mut self.child);
        let _ = self.child.wait();
    }
}

static CLIENT: Mutex<Option<Client>> = Mutex::new(None);

impl Client {
    fn connect(claude_cmd: &str) -> Result<Client, String> {
        let mut cmd = crate::launch::claude_command(claude_cmd, &[SERVER_ARG.to_string()]);
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        #[cfg(windows)]
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        for (k, v) in crate::shell_env::captured_env().to_inject() {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().map_err(|e| format!("spawn failed: {e}"))?;
        let stdin = child.stdin.take().ok_or("no stdin")?;
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let (tx, lines) = mpsc::channel();
        std::thread::Builder::new()
            .name("claude-chrome-mcp".into())
            .spawn(move || {
                for line in BufReader::new(stdout).lines() {
                    let Ok(line) = line else { break };
                    if tx.send(line).is_err() {
                        break;
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        let mut client = Client { claude_cmd: claude_cmd.to_string(), child, stdin, lines, next_id: 1 };
        client.request(
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "claude-eclipse-ide", "version": env!("CARGO_PKG_VERSION") }
            }),
        )?;
        client.send(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))?;
        Ok(client)
    }

    fn send(&mut self, msg: &Value) -> Result<(), String> {
        let mut line = msg.to_string();
        line.push('\n');
        self.stdin
            .write_all(line.as_bytes())
            .and_then(|_| self.stdin.flush())
            .map_err(|e| e.to_string())
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))?;
        let deadline = Instant::now() + CALL_TIMEOUT;
        loop {
            match self.lines.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(line) => {
                    let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
                    if v.get("id").and_then(Value::as_u64) != Some(id) {
                        continue;
                    }
                    if let Some(err) = v.get("error") {
                        return Err(err.to_string());
                    }
                    return Ok(v.get("result").cloned().unwrap_or(Value::Null));
                }
                Err(RecvTimeoutError::Timeout) => return Err(format!("{method} timed out")),
                Err(RecvTimeoutError::Disconnected) => return Err("server exited".into()),
            }
        }
    }
}

/// Calls one tool on the shared client, connecting first when there is none. A
/// failed call drops the client so the next one starts clean, and a client that
/// died since the last lookup gets one reconnect before the failure is reported.
fn call_tool(claude_cmd: &str, name: &str, arguments: Value) -> Result<Value, String> {
    let mut guard = CLIENT.lock().unwrap_or_else(|p| p.into_inner());
    if guard.as_ref().is_some_and(|c| c.claude_cmd != claude_cmd) {
        *guard = None;
    }
    let reused = guard.is_some();
    for attempt in 0..2 {
        if guard.is_none() {
            *guard = Some(Client::connect(claude_cmd)?);
        }
        let params = json!({ "name": name, "arguments": arguments });
        match guard.as_mut().unwrap().request("tools/call", params) {
            Ok(result) => return Ok(result),
            Err(e) => {
                *guard = None;
                if !(reused && attempt == 0) {
                    return Err(e);
                }
            }
        }
    }
    Err("unreachable".into())
}

/// Every text part of a tool result, joined.
fn result_text(result: &Value) -> String {
    match result.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts.iter().filter_map(|p| p.get("text")?.as_str()).collect(),
        _ => String::new(),
    }
}

/// The first text part of a tool result — where `tabs_context_mcp` puts its JSON.
fn first_text(result: &Value) -> Option<&str> {
    result.get("content")?.as_array()?.iter().find(|p| p["type"] == "text")?.get("text")?.as_str()
}

/// JS `String(v)` for the ids the tool returns, which arrive as numbers.
fn id_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

#[derive(Clone)]
struct Tab {
    group: String,
    id: String,
    title: String,
    url: String,
}

/// The tabs the last lookup found, for the plain `@` list — which is asked for on
/// every keystroke and must not wait on Chrome. `(claude_cmd, tabs)`.
static TABS_CACHE: Mutex<Option<(String, Vec<Tab>)>> = Mutex::new(None);
/// Set while a background refresh of [`TABS_CACHE`] is running, so a burst of
/// keystrokes starts one refresh rather than one per key.
static REFRESHING: AtomicBool = AtomicBool::new(false);

fn new_tab_row() -> Tab {
    Tab { group: String::new(), id: "0".into(), title: "new tab".into(), url: String::new() }
}

/// Looks the tabs up (blocking) and remembers the answer for [`cached_browser_tabs_json`].
fn browser_tabs(claude_cmd: &str) -> Vec<Tab> {
    let tabs = lookup_browser_tabs(claude_cmd);
    *TABS_CACHE.lock().unwrap_or_else(|p| p.into_inner()) = Some((claude_cmd.to_string(), tabs.clone()));
    tabs
}

fn lookup_browser_tabs(claude_cmd: &str) -> Vec<Tab> {
    let Ok(result) = call_tool(claude_cmd, "tabs_context_mcp", json!({ "createIfEmpty": false })) else {
        return Vec::new();
    };
    if result["isError"] == true {
        return if result_text(&result).contains("No tab available") { vec![new_tab_row()] } else { Vec::new() };
    }
    let Some(text) = first_text(&result) else { return Vec::new() };
    match serde_json::from_str::<Value>(text) {
        Ok(ctx) => {
            let group = id_string(&ctx["tabGroupId"]);
            let mut tabs: Vec<Tab> = ctx["availableTabs"]
                .as_array()
                .map(|a| a.as_slice())
                .unwrap_or_default()
                .iter()
                .map(|t| Tab {
                    group: group.clone(),
                    id: id_string(&t["tabId"]),
                    title: t["title"].as_str().unwrap_or_default().to_string(),
                    url: t["url"].as_str().unwrap_or_default().to_string(),
                })
                .collect();
            tabs.push(new_tab_row());
            tabs
        }
        Err(_) if text.contains("No MCP tab groups found.") => vec![new_tab_row()],
        Err(_) => Vec::new(),
    }
}

/// The `@browser:` rows for `query` (the whole typed word, `browser:` included),
/// shaped like the file rows: `{"path","name","type":"browser"}`. Blocking: asks
/// Chrome, as the extension does once the word starts with `browser:`.
pub fn browser_tabs_json(claude_cmd: &str, query: &str) -> String {
    tab_rows_json(browser_tabs(claude_cmd), query)
}

/// The same rows from the last lookup, without waiting — the extension's cached
/// path, used for the browser tabs a plain `@` list carries alongside the files.
/// Starts a refresh in the background, so the next keystroke sees fresher tabs.
pub fn cached_browser_tabs_json(claude_cmd: &str, query: &str) -> String {
    let tabs = match TABS_CACHE.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
        Some((cmd, tabs)) if cmd == claude_cmd => tabs.clone(),
        _ => Vec::new(),
    };
    if !REFRESHING.swap(true, Ordering::SeqCst) {
        let cmd = claude_cmd.to_string();
        let started = std::thread::Builder::new()
            .name("claude-chrome-tabs-refresh".into())
            .spawn(move || {
                browser_tabs(&cmd);
                REFRESHING.store(false, Ordering::SeqCst);
            });
        if started.is_err() {
            REFRESHING.store(false, Ordering::SeqCst);
        }
    }
    tab_rows_json(tabs, query)
}

fn tab_rows_json(tabs: Vec<Tab>, query: &str) -> String {
    let q = query.to_lowercase();
    let rows: Vec<Value> = tabs
        .into_iter()
        .filter(|t| {
            query.is_empty()
                || format!("browser:{}", t.title).to_lowercase().contains(&q)
                || t.url.to_lowercase().contains(&q)
        })
        .map(|t| {
            let path = if t.group.is_empty() && t.id == "0" {
                "browser:new_tab".to_string()
            } else {
                format!("browser:{}:{}:{}", t.group, t.id, t.url)
            };
            json!({ "path": path, "name": format!("browser:{}", t.title.replace(' ', "_")), "type": "browser" })
        })
        .collect();
    serde_json::to_string(&rows).unwrap_or_else(|_| "[]".to_string())
}

/// Opens a tab in Claude's tab group (creating the group when there is none).
/// Returns `(tabGroupId, tabId)`.
fn new_browser_tab(claude_cmd: &str) -> Option<(String, String)> {
    let result = call_tool(claude_cmd, "tabs_context_mcp", json!({ "createIfEmpty": true })).ok()?;
    if result["isError"] == true {
        return None;
    }
    let ctx: Value = serde_json::from_str(first_text(&result)?).ok()?;
    Some((id_string(&ctx["tabGroupId"]), context_tab_id(&ctx)))
}

/// The tab a `tabs_context_mcp` reply points at.
///
/// The reply names it `selectedTabId`, with the tabs themselves under
/// `availableTabs`; there is no bare `tabId`. Reading one left the mention with an
/// empty id, which dropped the whole `<browser>` block — and Claude, handed no tab,
/// opened its own after a failed `tabs_create_mcp` ("No tab group exists for this
/// session yet"). The tab list beside this has always read the right fields, which
/// is why it worked while this did not.
fn context_tab_id(ctx: &Value) -> String {
    let selected = id_string(&ctx["selectedTabId"]);
    if !selected.is_empty() {
        return selected;
    }
    let first = ctx["availableTabs"]
        .as_array()
        .and_then(|tabs| tabs.first())
        .map(|tab| id_string(&tab["tabId"]))
        .unwrap_or_default();
    if !first.is_empty() {
        return first;
    }
    // Whatever a future shape calls it, if it ever says so plainly.
    id_string(&ctx["tabId"])
}

/// The `claude-in-chrome` server config handed to the conversation's process.
pub fn server_config(claude_cmd: &str) -> Value {
    let (command, args) = crate::launch::stdio_server_command(claude_cmd, &[SERVER_ARG]);
    json!({ "type": "stdio", "command": command, "args": args })
}

/// The browser mentions in `message`: `None` asks for a new tab (`@browser`,
/// `@browser:new_tab`), `Some((group, tab, url))` names an existing one
/// (`@browser:<group>:<tab>:<url>`). Same forms the extension's pattern accepts.
fn browser_mentions(message: &str) -> Vec<Option<(String, String, String)>> {
    const AT: &str = "@browser";
    let mut out = Vec::new();
    let mut rest = message;
    while let Some(i) = rest.find(AT) {
        let after = &rest[i + AT.len()..];
        rest = after;
        match after.chars().next() {
            None => out.push(None),
            Some(c) if c.is_whitespace() => out.push(None),
            Some(':') => {
                let tail = &after[1..];
                let token = &tail[..tail.find(char::is_whitespace).unwrap_or(tail.len())];
                let named = token.split_once(':').and_then(|(group, r)| {
                    let (tab, url) = r.split_once(':')?;
                    (!tab.is_empty() && tab.bytes().all(|b| b.is_ascii_digit()))
                        .then(|| (group.to_string(), tab.to_string(), url.to_string()))
                });
                match named {
                    Some(n) => out.push(Some(n)),
                    None if token.starts_with("new_tab") => out.push(None),
                    None => {}
                }
            }
            Some(_) => {}
        }
    }
    out
}

/// The text blocks a message mentioning the browser carries, in the extension's
/// order: the instruction when `enable` reports the browser was just switched on
/// for this conversation (it returns 1), then one `<browser>` block per mention,
/// opening a tab for each mention that asks for a new one. `instruction` supplies
/// the instruction's text (see [`cli_instruction`]); without it the browser still
/// works, and Claude only goes without the extension's guidance.
pub fn browser_blocks(
    claude_cmd: &str,
    message: &str,
    enable: impl FnOnce(&Value) -> i32,
    instruction: impl FnOnce() -> Option<String>,
) -> Vec<Value> {
    let mentions = browser_mentions(message);
    if mentions.is_empty() {
        return Vec::new();
    }
    let mut blocks = Vec::new();
    if enable(&server_config(claude_cmd)) == 1 {
        if let Some(text) = instruction() {
            blocks.push(json!({
                "type": "text",
                "text": format!("<browser_instruction>{text}</browser_instruction>")
            }));
        }
    }
    for mention in mentions {
        let (group, tab, url) = match mention {
            Some((g, t, u)) if !(g.is_empty() && t == "0") => (g, t, u),
            other => {
                let url = other.map(|m| m.2).unwrap_or_default();
                let Some((g, t)) = new_browser_tab(claude_cmd) else { continue };
                (g, t, url)
            }
        };
        if group.is_empty() || tab.is_empty() {
            continue;
        }
        blocks.push(json!({
            "type": "text",
            "text": format!("<browser tabGroupId=\"{group}\" tabId=\"{tab}\">{url}</browser>")
        }));
    }
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tab_id_comes_from_the_reply_the_server_really_sends() {
        // Captured from `claude --claude-in-chrome-mcp`, tabs_context_mcp.
        let ctx: Value = serde_json::from_str(
            r#"{"availableTabs":[{"tabId":60923631,"title":"New Tab","url":"chrome://newtab/"}],
                "selectedTabId":60923631,"tabGroupId":1988695452}"#,
        )
        .unwrap();
        assert_eq!(id_string(&ctx["tabGroupId"]), "1988695452");
        assert_eq!(context_tab_id(&ctx), "60923631");

        // No selection: the first available tab stands in.
        let ctx: Value = serde_json::from_str(
            r#"{"availableTabs":[{"tabId":7,"title":"t","url":"u"}],"tabGroupId":3}"#,
        )
        .unwrap();
        assert_eq!(context_tab_id(&ctx), "7");

        // Nothing to go on: empty, and the caller drops the block rather than
        // inventing a tab.
        let ctx: Value = serde_json::from_str(r#"{"tabGroupId":3}"#).unwrap();
        assert!(context_tab_id(&ctx).is_empty());
    }

    #[test]
    fn mentions_follow_the_extension_forms() {
        assert_eq!(browser_mentions("open @browser now"), vec![None]);
        assert_eq!(browser_mentions("@browser"), vec![None]);
        assert_eq!(browser_mentions("@browser:new_tab go"), vec![None]);
        assert_eq!(
            browser_mentions("see @browser:771:12:https://example.com/a?b=c:d then"),
            vec![Some(("771".into(), "12".into(), "https://example.com/a?b=c:d".into()))]
        );
        assert!(browser_mentions("@browserx @browser:tab_title mail@example.com").is_empty());
    }

    #[test]
    fn no_mention_touches_nothing() {
        let mut called = false;
        let blocks = browser_blocks(
            "claude",
            "plain message",
            |_| {
                called = true;
                1
            },
            || panic!("no mention asks for the instruction"),
        );
        assert!(blocks.is_empty() && !called);
    }

    #[test]
    fn named_tab_gets_instruction_once_enabled() {
        let instruction = || Some(format!("{INSTRUCTION_HEAD}\nbody"));
        let blocks = browser_blocks(
            "claude",
            "@browser:9:4:https://x.dev",
            |cfg| {
                assert_eq!(cfg["type"], "stdio");
                1
            },
            instruction,
        );
        assert_eq!(blocks.len(), 2);
        let first = blocks[0]["text"].as_str().unwrap();
        assert!(first.starts_with("<browser_instruction># Claude in Chrome browser automation"));
        assert!(first.ends_with("</browser_instruction>"));
        assert_eq!(blocks[1]["text"], "<browser tabGroupId=\"9\" tabId=\"4\">https://x.dev</browser>");

        let again = browser_blocks("claude", "@browser:9:4:https://x.dev", |_| 0, instruction);
        assert_eq!(again.len(), 1);
    }

    #[test]
    fn without_the_instruction_the_mention_still_goes() {
        let blocks = browser_blocks("claude", "@browser:9:4:https://x.dev", |_| 1, || None);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0]["text"], "<browser tabGroupId=\"9\" tabId=\"4\">https://x.dev</browser>");
    }

    #[test]
    fn template_literal_is_evaluated_like_js() {
        // The shape the CLI bundles: string-literal slots, \u escapes, escaped backticks.
        let src = r#"`# Head
${'ToolSearch with query "select:a,b"'} and ${ "xA\"" } — \`tick\` \${kept} \u{1F600}😀 end`,next=`other`"#;
        assert_eq!(
            eval_template(src).as_deref(),
            Some("# Head\nToolSearch with query \"select:a,b\" and xA\" \u{2014} `tick` ${kept} \u{1F600}\u{1F600} end")
        );
        // Anything that isn't a plain literal is refused rather than guessed at.
        assert_eq!(eval_template("`a ${name} b`"), None);
        assert_eq!(eval_template("`no closing backtick"), None);
        assert_eq!(eval_template(r"`bad \u12 escape`"), None);
        assert_eq!(eval_template("not a template"), None);
    }

    #[test]
    fn a_match_split_across_two_reads_is_found() {
        let hay = b"0123456789`# Head and more".to_vec();
        for chunk in [1, 3, 4, 11, 64] {
            assert_eq!(find_in_reader(&mut &hay[..], b"`# Head", chunk), Some(10), "chunk {chunk}");
        }
        assert_eq!(find_in_reader(&mut &hay[..], b"`# Nope", 4), None);
    }

    #[test]
    fn instruction_is_read_out_of_a_program_file() {
        let body = "call tabs_context_mcp first. ".repeat(60);
        let mut file = vec![0xFFu8, 0x00, 0xC3, 0x28]; // not UTF-8, like a real binary
        file.extend_from_slice(b"var a=1;var O9e=`");
        file.extend_from_slice(format!("{INSTRUCTION_HEAD}\n{body}${{'x'}} \\u2014 {INSTRUCTION_TAIL}`,P=`next`").as_bytes());
        file.extend_from_slice(&[0xFE, 0xFF]);
        let path = std::env::temp_dir().join("claude-eclipse-instruction-fixture.bin");
        std::fs::write(&path, &file).unwrap();

        let text = read_instruction(&path);
        let short = std::env::temp_dir().join("claude-eclipse-instruction-short.bin");
        std::fs::write(&short, format!("`{INSTRUCTION_HEAD}\n{INSTRUCTION_TAIL}`")).unwrap();
        let too_short = read_instruction(&short);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&short);

        assert_eq!(text, Some(format!("{INSTRUCTION_HEAD}\n{body}x \u{2014} {INSTRUCTION_TAIL}")));
        assert_eq!(too_short, None, "a fragment is not the instruction");
    }

    /// Opt-in: reads the Claude Code CLI installed on this machine. Run with
    /// `cargo test --release -- --ignored installed_cli`.
    #[test]
    #[ignore = "reads the installed Claude Code CLI"]
    fn installed_cli_carries_the_instruction() {
        let started = std::time::Instant::now();
        let text = cli_instruction("claude").expect("instruction not found in the installed CLI");
        eprintln!("instruction: {} bytes, read in {:?}", text.len(), started.elapsed());
        // Written out so it can be compared byte for byte with the extension's text.
        let dump = std::env::temp_dir().join("claude-eclipse-cli-instruction.txt");
        std::fs::write(&dump, &text).unwrap();
        eprintln!("written to {}", dump.display());
        let cached = std::time::Instant::now();
        assert_eq!(cli_instruction("claude").as_deref(), Some(text.as_str()));
        eprintln!("cached: {:?}", cached.elapsed());
    }
}
