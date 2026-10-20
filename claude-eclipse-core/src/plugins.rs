//! The plugins window (the `/` menu → Manage plugins, or `/plugins`), which reaches Claude
//! Code's plugins the way the VS Code extension reaches them: by running the CLI's own
//! `claude plugin …` commands, in the conversation's folder, so the files they write are
//! written by the thing that owns their format.
//!
//! The list, the marketplaces and every change are those commands; the CLI has no control
//! request for them (its only plugin ones are `plugin_install` and `reload_plugins`, and the
//! second is what the page sends once a change is made, over the MCP window's channel).
//!
//! What comes back is cut down to what the window draws: the catalogue of a marketplace is
//! thousands of entries, and each description is a paragraph.

use std::io::{Read, Write};
use std::process::Stdio;
use std::time::{Duration, Instant};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// How long listing what is installed or configured may take: it reads local files.
const LOCAL_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a command that fetches may take: the catalogue, a marketplace, a plugin.
const NETWORK_TIMEOUT: Duration = Duration::from_secs(120);

/// A description is cut to this many characters. The window shows two lines of it.
const DESCRIPTION_MAX: usize = 300;

/// What a command's output is turned into for the page.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Answer {
    /// `claude plugin list --json`: what is installed.
    Installed,
    /// `claude plugin list --json --available`: what is installed and what could be.
    Catalogue,
    /// `claude plugin marketplace list --json`.
    Marketplaces,
    /// `claude plugin configure --json`: a plugin's options.
    Options,
    /// `claude plugin configure --json --values-stdin`: the options that were saved.
    Saved,
    /// A change: nothing to say but that it was made.
    Done,
}

/// One command to run.
#[derive(Debug, PartialEq)]
struct Request {
    args: Vec<String>,
    stdin: Option<String>,
    timeout: Duration,
    answer: Answer,
}

/// Runs one of the window's requests and returns `{"token","ok":true,"response"}` or
/// `{"token","ok":false,"error"}`, the error being what the CLI said. **Blocking**, up to two
/// minutes.
///
/// * `{"op":"list","available":bool}`, `{"op":"marketplaces"}`
/// * `{"op":"install"|"update"|"enable"|"disable","id","scope"}`, `{"op":"uninstall","id","scope","keepData"}`
/// * `{"op":"marketplaceAdd","source"}`, `{"op":"marketplaceRemove","name"}`, `{"op":"marketplaceUpdate","name"?}`
/// * `{"op":"options","id"}`, `{"op":"saveOptions","id","values":{key:string}}`
pub fn run(claude_cmd: &str, cwd: &str, token: &str, op_json: &str) -> String {
    let result = serde_json::from_str::<serde_json::Value>(op_json)
        .map_err(|_| "Invalid request.".to_string())
        .and_then(|op| request(&op))
        .and_then(|req| {
            let out = run_cli(claude_cmd, cwd, &req.args, req.stdin.as_deref(), req.timeout)?;
            answer(req.answer, &out)
        });
    match result {
        Ok(response) => serde_json::json!({ "token": token, "ok": true, "response": response }),
        Err(e) => serde_json::json!({ "token": token, "ok": false, "error": e }),
    }
    .to_string()
}

/// A plugin id (`name@marketplace`) or a marketplace name: what the CLI takes as one
/// argument. Never one that begins like an option, since it ends up on a command line.
fn check_id(id: &str, what: &str) -> Result<(), String> {
    let fits = !id.is_empty()
        && id.len() <= 200
        && !id.starts_with('-')
        && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '@' | ':' | '/' | '+'));
    if fits { Ok(()) } else { Err(format!("Invalid {what}: {id}")) }
}

fn check_scope(scope: &str) -> Result<(), String> {
    match scope {
        "user" | "project" | "local" => Ok(()),
        _ => Err(format!("Unknown scope {scope}.")),
    }
}

/// A marketplace source as typed: a GitHub repo, a URL or a path. Whatever it says, it is
/// one argument, and not one that could be taken for an option.
fn check_source(source: &str) -> Result<(), String> {
    if source.is_empty() {
        return Err("Enter a GitHub repo, URL, or path.".into());
    }
    if source.starts_with('-') || source.len() > 500 || source.chars().any(char::is_control) {
        return Err(format!("Invalid marketplace source: {source}"));
    }
    Ok(())
}

fn strs(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

/// The command a request of the window stands for. The page has checked all of this to word
/// its own errors; it is checked again here because it becomes the arguments of a process.
fn request(op: &serde_json::Value) -> Result<Request, String> {
    let text = |k: &str| op[k].as_str().unwrap_or("").trim().to_string();
    let plain = |args: Vec<String>, timeout, answer| Ok(Request { args, stdin: None, timeout, answer });
    match op["op"].as_str() {
        Some("list") => {
            if op["available"].as_bool() == Some(true) {
                plain(strs(&["plugin", "list", "--json", "--available"]), NETWORK_TIMEOUT, Answer::Catalogue)
            } else {
                plain(strs(&["plugin", "list", "--json"]), LOCAL_TIMEOUT, Answer::Installed)
            }
        }
        Some("marketplaces") => plain(strs(&["plugin", "marketplace", "list", "--json"]), LOCAL_TIMEOUT, Answer::Marketplaces),
        Some(verb @ ("install" | "update" | "enable" | "disable" | "uninstall")) => {
            let (id, scope) = (text("id"), text("scope"));
            check_id(&id, "plugin")?;
            check_scope(&scope)?;
            let mut args = strs(&["plugin", verb, "--scope", &scope]);
            if verb == "uninstall" && op["keepData"].as_bool() == Some(true) {
                args.push("--keep-data".into());
            }
            args.push(id);
            let timeout = if matches!(verb, "install" | "update") { NETWORK_TIMEOUT } else { LOCAL_TIMEOUT };
            plain(args, timeout, Answer::Done)
        }
        Some("marketplaceAdd") => {
            let source = text("source");
            check_source(&source)?;
            plain(vec!["plugin".into(), "marketplace".into(), "add".into(), source], NETWORK_TIMEOUT, Answer::Done)
        }
        Some("marketplaceRemove") => {
            let name = text("name");
            check_id(&name, "marketplace")?;
            plain(vec!["plugin".into(), "marketplace".into(), "remove".into(), name], LOCAL_TIMEOUT, Answer::Done)
        }
        Some("marketplaceUpdate") => {
            let name = text("name");
            let mut args = strs(&["plugin", "marketplace", "update"]);
            if !name.is_empty() {
                check_id(&name, "marketplace")?;
                args.push(name);
            }
            plain(args, NETWORK_TIMEOUT, Answer::Done)
        }
        Some("options") => {
            let id = text("id");
            check_id(&id, "plugin")?;
            plain(vec!["plugin".into(), "configure".into(), "--json".into(), id], NETWORK_TIMEOUT, Answer::Options)
        }
        Some("saveOptions") => {
            let id = text("id");
            check_id(&id, "plugin")?;
            // Single-line strings, as the command asks.
            let values = op["values"].as_object().ok_or("No values to save.")?;
            if values.values().any(|v| v.as_str().map_or(true, |s| s.contains(['\n', '\r']))) {
                return Err("Option values must be single-line text.".into());
            }
            Ok(Request {
                args: vec!["plugin".into(), "configure".into(), "--json".into(), "--values-stdin".into(), id],
                stdin: Some(serde_json::Value::Object(values.clone()).to_string()),
                timeout: NETWORK_TIMEOUT,
                answer: Answer::Saved,
            })
        }
        _ => Err("Unknown plugin operation.".into()),
    }
}

/// `claude <args>` run in `cwd`, its standard output returned. **Blocking**, up to `timeout`.
/// The error is what the CLI said, standard error first.
pub(crate) fn run_cli(
    claude_cmd: &str,
    cwd: &str,
    args: &[String],
    stdin: Option<&str>,
    timeout: Duration,
) -> Result<String, String> {
    run_cli_status(claude_cmd, cwd, args, stdin, timeout).map_err(|f| f.said)
}

/// Why [`run_cli_status`] failed: what to tell the user, and how the CLI exited when it ran and
/// failed (None when it never ran or did not finish).
pub(crate) struct CliFailure {
    pub(crate) said: String,
    pub(crate) status: Option<std::process::ExitStatus>,
}

impl From<String> for CliFailure {
    fn from(said: String) -> Self {
        CliFailure { said, status: None }
    }
}

/// [`run_cli`], with the exit status of a CLI that ran and failed.
pub(crate) fn run_cli_status(
    claude_cmd: &str,
    cwd: &str,
    args: &[String],
    stdin: Option<&str>,
    timeout: Duration,
) -> Result<String, CliFailure> {
    if cwd.is_empty() || !std::path::Path::new(cwd).is_dir() {
        return Err(format!("Working directory not found: {cwd}").into());
    }
    let mut cmd = crate::launch::claude_command(claude_cmd, args);
    cmd.current_dir(cwd)
        .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(windows)]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW

    for (k, v) in crate::shell_env::captured_env().to_inject() {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().map_err(|e| format!("Could not run Claude Code: {e}"))?;

    if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
        // Written on a thread of its own and closed: the command reads to the end.
        let text = text.to_string();
        std::thread::spawn(move || {
            let _ = pipe.write_all(text.as_bytes());
        });
    }

    // Both pipes drained while it runs, so a chatty CLI cannot fill one and stall.
    let drain = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut p) = pipe {
                let _ = p.read_to_end(&mut bytes);
            }
            String::from_utf8_lossy(&bytes).into_owned()
        })
    };
    let out = drain(child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let err = drain(child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>));

    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < timeout => std::thread::sleep(Duration::from_millis(50)),
            Ok(None) => {
                crate::launch::kill_process_tree(&mut child);
                let _ = child.wait();
                return Err("Claude Code did not finish in time.".to_string().into());
            }
            Err(e) => return Err(format!("Could not run Claude Code: {e}").into()),
        }
    };
    let stdout = out.join().unwrap_or_default();
    let stderr = err.join().unwrap_or_default();
    if status.success() {
        return Ok(stdout);
    }
    let said = if stderr.trim().is_empty() { stdout.trim() } else { stderr.trim() };
    let said = if said.is_empty() { format!("Claude Code exited with {status}.") } else { said.to_string() };
    Err(CliFailure { said, status: Some(status) })
}

// ---------------------------------------------------------------------------
// What the commands answer
// ---------------------------------------------------------------------------

/// The JSON a command printed, from its first brace or bracket: a CLI that has something to
/// say first (a notice of an update) does not make the answer unreadable.
fn parse(out: &str) -> Result<serde_json::Value, String> {
    let start = out.find(['{', '[']).ok_or("Claude Code gave no answer.")?;
    serde_json::from_str(&out[start..]).map_err(|_| "Claude Code's answer could not be read.".to_string())
}

fn answer(kind: Answer, out: &str) -> Result<serde_json::Value, String> {
    match kind {
        Answer::Done => Ok(serde_json::json!({})),
        Answer::Installed => Ok(serde_json::json!({ "installed": installed(&parse(out)?) })),
        Answer::Catalogue => {
            let all = parse(out)?;
            Ok(serde_json::json!({ "installed": installed(&all["installed"]), "available": available(&all["available"]) }))
        }
        Answer::Marketplaces => Ok(serde_json::json!({ "marketplaces": marketplaces(&parse(out)?) })),
        Answer::Options => {
            let o = parse(out)?;
            // What the form is drawn from, as the command gives it.
            Ok(serde_json::json!({
                "schema": o["schema"], "inputs": o["inputs"], "choices": o["choices"],
                "configured": o["configured"], "unconfigured": o["unconfigured"],
            }))
        }
        Answer::Saved => Ok(serde_json::json!({ "saved": parse(out).map(|s| s["saved"].clone()).unwrap_or_default() })),
    }
}

/// What the window shows of an installed plugin: its id, version, scope, whether it is on, and
/// the names of the MCP servers it brings.
fn installed(list: &serde_json::Value) -> Vec<serde_json::Value> {
    list.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|p| {
                    let id = p["id"].as_str()?;
                    let servers: Vec<&String> = p["mcpServers"].as_object().map(|m| m.keys().collect()).unwrap_or_default();
                    Some(serde_json::json!({
                        "id": id,
                        "version": p["version"],
                        "scope": p["scope"],
                        "enabled": p["enabled"].as_bool().unwrap_or(false),
                        "mcpServers": servers,
                    }))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// A text of at most `max` characters, cut on a character boundary and marked.
fn cut(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max {
        return text.to_string();
    }
    let head: String = text.chars().take(max).collect();
    format!("{}…", head.trim_end())
}

/// What the window shows of an installable plugin, the most installed first. Its source is
/// kept to the parts a link is made of.
fn available(list: &serde_json::Value) -> Vec<serde_json::Value> {
    let mut out: Vec<serde_json::Value> = list
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|p| {
                    let id = p["pluginId"].as_str()?;
                    let source = match &p["source"] {
                        serde_json::Value::Object(o) => {
                            let keep = ["source", "url", "repo", "path", "ref", "package"];
                            serde_json::Value::Object(o.iter().filter(|(k, _)| keep.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect())
                        }
                        other => other.clone(),
                    };
                    Some(serde_json::json!({
                        "pluginId": id,
                        "name": p["name"].as_str().unwrap_or(id),
                        "description": cut(p["description"].as_str().unwrap_or(""), DESCRIPTION_MAX),
                        "marketplace": p["marketplaceName"],
                        "installCount": p["installCount"],
                        "source": source,
                    }))
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by_key(|p| std::cmp::Reverse(p["installCount"].as_u64().unwrap_or(0)));
    out
}

fn marketplaces(list: &serde_json::Value) -> Vec<serde_json::Value> {
    list.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|m| {
                    let name = m["name"].as_str()?;
                    Some(serde_json::json!({
                        "name": name, "source": m["source"], "repo": m["repo"], "url": m["url"],
                        "path": m["path"], "package": m["package"],
                    }))
                })
                .collect()
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn args(op: serde_json::Value) -> Result<Vec<String>, String> {
        request(&op).map(|r| r.args)
    }

    #[test]
    fn the_lists_are_the_clis_own() {
        assert_eq!(args(json!({"op":"list"})).unwrap(), ["plugin", "list", "--json"]);
        assert_eq!(args(json!({"op":"list","available":true})).unwrap(), ["plugin", "list", "--json", "--available"]);
        assert_eq!(args(json!({"op":"marketplaces"})).unwrap(), ["plugin", "marketplace", "list", "--json"]);
    }

    #[test]
    fn a_change_names_its_scope_and_ends_on_the_plugin() {
        assert_eq!(args(json!({"op":"install","id":"frontend-design@claude-plugins-official","scope":"user"})).unwrap(),
                   ["plugin", "install", "--scope", "user", "frontend-design@claude-plugins-official"]);
        assert_eq!(args(json!({"op":"uninstall","id":"a@b","scope":"project","keepData":true})).unwrap(),
                   ["plugin", "uninstall", "--scope", "project", "--keep-data", "a@b"]);
        assert_eq!(args(json!({"op":"disable","id":"a@b","scope":"local"})).unwrap(), ["plugin", "disable", "--scope", "local", "a@b"]);
    }

    #[test]
    fn marketplaces_are_added_removed_and_refreshed() {
        assert_eq!(args(json!({"op":"marketplaceAdd","source":" anthropics/claude-plugins-official "})).unwrap(),
                   ["plugin", "marketplace", "add", "anthropics/claude-plugins-official"]);
        assert_eq!(args(json!({"op":"marketplaceRemove","name":"x"})).unwrap(), ["plugin", "marketplace", "remove", "x"]);
        assert_eq!(args(json!({"op":"marketplaceUpdate"})).unwrap(), ["plugin", "marketplace", "update"]);
        assert_eq!(args(json!({"op":"marketplaceUpdate","name":"x"})).unwrap(), ["plugin", "marketplace", "update", "x"]);
    }

    #[test]
    fn nothing_that_could_be_an_option_or_is_not_a_name_reaches_the_command_line() {
        for bad in ["", "--scope", "-x@y", "a b", "a;b", "a\nb"] {
            assert!(args(json!({"op":"install","id":bad,"scope":"user"})).is_err(), "{bad:?}");
            assert!(args(json!({"op":"marketplaceRemove","name":bad})).is_err(), "{bad:?}");
        }
        assert!(args(json!({"op":"install","id":"a@b","scope":"managed"})).is_err());
        for bad in ["", "--help", "a\nb"] {
            assert!(args(json!({"op":"marketplaceAdd","source":bad})).is_err(), "{bad:?}");
        }
        assert!(args(json!({"op":"rm -rf"})).is_err());
    }

    #[test]
    fn options_are_saved_on_standard_input_as_single_lines() {
        let r = request(&json!({"op":"saveOptions","id":"a@b","values":{"token":"s3cret","region":"eu"}})).unwrap();
        assert_eq!(r.args, ["plugin", "configure", "--json", "--values-stdin", "a@b"]);
        let said: serde_json::Value = serde_json::from_str(&r.stdin.unwrap()).unwrap();
        assert_eq!(said, json!({"token":"s3cret","region":"eu"}));
        assert!(request(&json!({"op":"saveOptions","id":"a@b","values":{"k":"two\nlines"}})).is_err());
        assert!(request(&json!({"op":"saveOptions","id":"a@b","values":{"k":1}})).is_err());
    }

    #[test]
    fn what_is_installed_is_cut_down_to_what_the_window_draws() {
        let out = r#"[{"id":"playwright@official","version":"1","scope":"user","enabled":true,"installPath":"C:\\x",
                       "mcpServers":{"playwright":{"command":"npx"}},"projectEnabled":false},
                      {"id":"s@m","scope":"user","enabled":false}]"#;
        let got = answer(Answer::Installed, out).unwrap();
        assert_eq!(got["installed"][0], json!({"id":"playwright@official","version":"1","scope":"user","enabled":true,"mcpServers":["playwright"]}));
        assert_eq!(got["installed"][1]["enabled"], false);
    }

    #[test]
    fn the_catalogue_is_cut_and_ordered_by_installs() {
        let long = "x".repeat(400);
        let out = json!({
            "installed": [],
            "available": [
                {"pluginId":"a@m","name":"a","description":"short","marketplaceName":"m","installCount":5,"source":"./plugins/a"},
                {"pluginId":"b@m","name":"b","description":long,"marketplaceName":"m","installCount":900,
                 "source":{"source":"git-subdir","url":"https://g/x.git","path":"p","ref":"v1","sha":"deadbeef"}},
            ]
        })
        .to_string();
        let got = answer(Answer::Catalogue, &out).unwrap();
        assert_eq!(got["available"][0]["pluginId"], "b@m", "most installed first");
        assert_eq!(got["available"][0]["description"].as_str().unwrap().chars().count(), DESCRIPTION_MAX + 1);
        assert!(got["available"][0]["source"].get("sha").is_none());
        assert_eq!(got["available"][1]["source"], "./plugins/a");
    }

    #[test]
    fn an_answer_may_follow_a_notice() {
        let got = answer(Answer::Marketplaces, "Update available\n[{\"name\":\"m\",\"source\":\"github\",\"repo\":\"o/r\"}]").unwrap();
        assert_eq!(got["marketplaces"][0]["repo"], "o/r");
        assert!(answer(Answer::Installed, "nothing here").is_err());
        assert!(answer(Answer::Installed, "[not json").is_err());
    }

    #[test]
    fn a_description_is_cut_on_a_character_boundary() {
        assert_eq!(cut(&"é".repeat(10), 4), "éééé…");
        assert_eq!(cut("  short ", 10), "short");
    }
}
