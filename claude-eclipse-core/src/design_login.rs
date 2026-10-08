//! The Claude Design sign-in.
//!
//! `claude design-login --json` is a conversation in lines, and the VS Code panel's
//! Claude Design dialog is drawn from it; this is that conversation for the Claude GUI.
//! The CLI prints the pages to sign in on (`pages`), waits for the browser to finish or
//! for a code pasted back on its stdin, and prints how it ended (`done`). Whether the
//! authorization is already there is the same subcommand with `--status`, run to
//! completion like the CLI's edits ([`crate::cli_ask::edit`]).
//!
//! One sign-in at a time: the authorization is the computer's, not a tab's.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// How long a sign-in may take: the extension's limit. Its dialog says five minutes.
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(360);

/// How long the CLI is given to leave by itself once its stdin is closed.
const EXIT_GRACE: Duration = Duration::from_secs(5);

/// How much of what the CLI says on stderr is kept for a failure's message.
const STDERR_KEPT: usize = 4096;

const TIMED_OUT: &str = "The sign-in timed out.";
const COULD_NOT_START: &str = "The sign-in could not start. Check that Claude Code is installed, then try again.";
const ALREADY_WAITING: &str = "A Claude Design sign-in is already waiting on the browser.";
const NONE_WAITING: &str = "No Claude Design sign-in is waiting.";

/// What the CLI says while a sign-in runs.
#[derive(Debug, Clone, PartialEq)]
enum Event {
    /// Where to sign in: the page that finishes by itself, the page that ends on a
    /// code to paste back, and whether the second is the one to open.
    Pages { url: String, manual_url: String, manual_first: bool },
    /// How it ended.
    Done { ok: bool, message: Option<String> },
}

/// The event on one line of the CLI's output, or `None` for a line that is not one.
fn event(line: &str) -> Option<Event> {
    let said: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    match said["event"].as_str()? {
        "pages" => Some(Event::Pages {
            url: said["url"].as_str()?.to_string(),
            manual_url: said["manual_url"].as_str()?.to_string(),
            manual_first: said["manual_first"].as_bool() == Some(true),
        }),
        "done" => Some(Event::Done { ok: said["ok"].as_bool()?, message: said["message"].as_str().map(String::from) }),
        _ => None,
    }
}

/// The line that hands the CLI a pasted authorization code, or why `code` is not one.
fn code_line(code: &str) -> Result<String, String> {
    let code = code.trim();
    if code.is_empty() || code.len() > 4096 {
        return Err("That is not an authorization code.".into());
    }
    if !code.contains('#') {
        return Err("Paste the whole code from the page, including the part after the #.".into());
    }
    Ok(serde_json::json!({ "code": code }).to_string())
}

/// What is said of a sign-in whose process went without saying how it ended.
fn ended_message(stderr: &str, exit_code: Option<i32>) -> String {
    let said = stderr.trim();
    if !said.is_empty() {
        return said.to_string();
    }
    let how = exit_code.map_or_else(|| "stopped".to_string(), |code| format!("exit code {code}"));
    format!("The sign-in ended before it finished ({how}). Try again.")
}

/// How far a sign-in has got.
#[derive(Default)]
struct Progress {
    pages: Option<Event>,
    /// Set once: whether it succeeded, and what the CLI said if it said anything.
    outcome: Option<(bool, Option<String>)>,
}

/// One running `claude design-login --json`.
struct SignIn {
    progress: Mutex<Progress>,
    changed: Condvar,
    /// Open while a code may still be pasted; closing it is how the CLI is told to stop.
    stdin: Mutex<Option<ChildStdin>>,
    child: Mutex<Child>,
}

impl SignIn {
    /// Records how it ended. The first ending stands.
    fn finish(&self, ok: bool, message: Option<String>) {
        let mut progress = self.progress.lock().unwrap();
        if progress.outcome.is_none() {
            progress.outcome = Some((ok, message));
        }
        drop(progress);
        self.changed.notify_all();
    }

    /// Waits until the CLI has named its pages or the sign-in has ended, whichever is first.
    fn started(&self) -> Result<Event, Option<String>> {
        let mut progress = self.progress.lock().unwrap();
        loop {
            if let Some(pages) = &progress.pages {
                return Ok(pages.clone());
            }
            if let Some((_, message)) = &progress.outcome {
                return Err(message.clone());
            }
            progress = self.changed.wait(progress).unwrap();
        }
    }

    /// Waits until the sign-in has ended.
    fn outcome(&self) -> (bool, Option<String>) {
        let mut progress = self.progress.lock().unwrap();
        loop {
            if let Some(outcome) = &progress.outcome {
                return outcome.clone();
            }
            progress = self.changed.wait(progress).unwrap();
        }
    }

    /// Hands the CLI one line on its stdin. False when it is no longer listening.
    fn say(&self, line: &str) -> bool {
        let mut stdin = self.stdin.lock().unwrap();
        stdin.as_mut().is_some_and(|pipe| writeln!(pipe, "{line}").and_then(|()| pipe.flush()).is_ok())
    }

    /// Whether the process has gone, and with what exit code if it had one.
    fn exited(&self) -> Option<Option<i32>> {
        self.child.lock().unwrap().try_wait().ok().flatten().map(|status| status.code())
    }
}

/// Starts `cmd` as a sign-in that may take `limit`, and follows it on threads of its
/// own until it has ended and its process is gone.
fn drive(mut cmd: Command, limit: Duration) -> std::io::Result<Arc<SignIn>> {
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    let (stdin, stdout, stderr) = (child.stdin.take(), child.stdout.take(), child.stderr.take());
    let run = Arc::new(SignIn {
        progress: Mutex::new(Progress::default()),
        changed: Condvar::new(),
        stdin: Mutex::new(stdin),
        child: Mutex::new(child),
    });

    // What it says on stderr, the end of it kept: the reason when it fails.
    let complaint = Arc::new(Mutex::new(String::new()));
    let heard_out = Arc::new(AtomicBool::new(stderr.is_none()));
    if let Some(mut pipe) = stderr {
        let complaint = Arc::clone(&complaint);
        let heard_out = Arc::clone(&heard_out);
        let _ = std::thread::Builder::new().name("claude-design-login-err".into()).spawn(move || {
            let mut chunk = [0u8; 1024];
            while let Ok(n) = pipe.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                let mut kept = complaint.lock().unwrap();
                kept.push_str(&String::from_utf8_lossy(&chunk[..n]));
                if kept.len() > STDERR_KEPT {
                    let mut cut = kept.len() - STDERR_KEPT;
                    while !kept.is_char_boundary(cut) {
                        cut += 1;
                    }
                    kept.drain(..cut);
                }
            }
            heard_out.store(true, Ordering::SeqCst);
        });
    }

    // What it says on stdout, a line at a time.
    let reader = Arc::clone(&run);
    let _ = std::thread::Builder::new().name("claude-design-login-out".into()).spawn(move || {
        if let Some(stdout) = stdout {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                match event(&line) {
                    Some(pages @ Event::Pages { .. }) => {
                        reader.progress.lock().unwrap().pages.get_or_insert(pages);
                        reader.changed.notify_all();
                    }
                    Some(Event::Done { ok, message }) => reader.finish(ok, message),
                    None => {}
                }
            }
        }
        // It has stopped talking. If it never said how it ended, its exit says so.
        let since = Instant::now();
        let mut exit = reader.exited();
        while exit.is_none() && since.elapsed() < EXIT_GRACE {
            std::thread::sleep(Duration::from_millis(25));
            exit = reader.exited();
        }
        // And what it said on the way out is given a moment to arrive.
        while !heard_out.load(Ordering::SeqCst) && since.elapsed() < EXIT_GRACE + Duration::from_secs(1) {
            std::thread::sleep(Duration::from_millis(10));
        }
        let said = complaint.lock().unwrap().clone();
        reader.finish(false, Some(ended_message(&said, exit.flatten())));
    });

    // The limit, and seeing the process out once the sign-in has ended any way at all.
    let keeper = Arc::clone(&run);
    let _ = std::thread::Builder::new().name("claude-design-login-end".into()).spawn(move || {
        let deadline = Instant::now() + limit;
        let mut progress = keeper.progress.lock().unwrap();
        while progress.outcome.is_none() {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            progress = keeper.changed.wait_timeout(progress, left).unwrap().0;
        }
        drop(progress);
        keeper.finish(false, Some(TIMED_OUT.into()));
        keeper.stdin.lock().unwrap().take();
        let since = Instant::now();
        while keeper.exited().is_none() && since.elapsed() < EXIT_GRACE {
            std::thread::sleep(Duration::from_millis(50));
        }
        let mut child = keeper.child.lock().unwrap();
        if !matches!(child.try_wait(), Ok(Some(_))) {
            crate::launch::kill_process_tree(&mut child);
            let _ = child.wait();
        }
    });
    Ok(run)
}

/// The sign-in in progress, if one is.
static RUN: Mutex<Option<Arc<SignIn>>> = Mutex::new(None);

/// The arguments the CLI is started with.
fn args() -> Vec<String> {
    ["design-login", "--json"].map(String::from).to_vec()
}

fn failed(message: &str) -> String {
    serde_json::json!({ "ok": false, "message": message }).to_string()
}

/// What `start` answers: the pages to open, or why there are none.
fn start_json(started: Result<Event, Option<String>>) -> String {
    match started {
        Ok(Event::Pages { url, manual_url, manual_first }) => {
            serde_json::json!({ "ok": true, "url": url, "manualUrl": manual_url, "manualFirst": manual_first }).to_string()
        }
        Ok(Event::Done { .. }) => failed(COULD_NOT_START),
        Err(message) => failed(message.as_deref().unwrap_or(COULD_NOT_START)),
    }
}

/// What `wait` answers: whether the sign-in succeeded, and what the CLI said if anything.
fn outcome_json(outcome: (bool, Option<String>)) -> String {
    match outcome {
        (ok, Some(message)) => serde_json::json!({ "ok": ok, "message": message }),
        (ok, None) => serde_json::json!({ "ok": ok }),
    }
    .to_string()
}

fn start(claude_cmd: &str, cwd: &str) -> String {
    let mut current = RUN.lock().unwrap();
    if current.as_ref().is_some_and(|run| run.progress.lock().unwrap().outcome.is_none()) {
        return failed(ALREADY_WAITING);
    }
    if cwd.is_empty() || !std::path::Path::new(cwd).is_dir() {
        return failed(&format!("Working directory not found: {cwd}"));
    }
    let mut cmd = crate::launch::claude_command(claude_cmd, &args());
    cmd.current_dir(cwd);

    #[cfg(windows)]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW

    for (k, v) in crate::shell_env::captured_env().to_inject() {
        cmd.env(k, v);
    }
    // As for the CLI's edits (cli_ask.rs): not an editor started inside a session.
    cmd.env_remove("CLAUDECODE").env_remove("CLAUDE_CODE_CHILD_SESSION");

    let run = match drive(cmd, SIGN_IN_TIMEOUT) {
        Ok(run) => run,
        Err(e) => {
            if crate::is_debug() {
                eprintln!("[design-login] could not be started: {e}");
            }
            return failed(COULD_NOT_START);
        }
    };
    *current = Some(Arc::clone(&run));
    drop(current);
    let started = run.started();
    if crate::is_debug() {
        // Never the pages themselves: their addresses carry the sign-in's secrets.
        eprintln!("[design-login] started: {}", if started.is_ok() { "pages named" } else { "ended before naming its pages" });
    }
    start_json(started)
}

fn wait() -> String {
    let Some(run) = RUN.lock().unwrap().clone() else { return failed(NONE_WAITING) };
    let outcome = run.outcome();
    let mut current = RUN.lock().unwrap();
    if current.as_ref().is_some_and(|now| Arc::ptr_eq(now, &run)) {
        *current = None;
    }
    if crate::is_debug() {
        eprintln!("[design-login] ended: ok={} {}", outcome.0, outcome.1.as_deref().unwrap_or(""));
    }
    outcome_json(outcome)
}

fn code(code: &str) -> String {
    let line = match code_line(code) {
        Ok(line) => line,
        Err(why) => return failed(&why),
    };
    let Some(run) = RUN.lock().unwrap().clone() else { return failed("No Claude Design sign-in is waiting for a code.") };
    if run.say(&line) {
        serde_json::json!({ "ok": true }).to_string()
    } else {
        failed("No Claude Design sign-in is waiting for a code.")
    }
}

fn cancel() -> String {
    if let Some(run) = RUN.lock().unwrap().take() {
        run.finish(false, None);
    }
    serde_json::json!({ "ok": true }).to_string()
}

/// One step of the Claude Design sign-in, by name:
///
/// * `start` starts the CLI in `cwd` and answers once it has named its pages:
///   `{"ok":true,"url","manualUrl","manualFirst"}`. **Blocking**, a few seconds.
/// * `wait` answers when the sign-in has ended: `{"ok","message"?}`. **Blocking**, up
///   to six minutes.
/// * `code` hands over an authorization code pasted from the page (`arg`).
/// * `cancel` ends it.
///
/// Every failure is `{"ok":false,"message"}`.
pub fn run(claude_cmd: &str, cwd: &str, op: &str, arg: &str) -> String {
    match op {
        "start" => start(claude_cmd, cwd),
        "wait" => wait(),
        "code" => code(arg),
        "cancel" => cancel(),
        _ => failed("Invalid request."),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parsed(s: &str) -> serde_json::Value {
        serde_json::from_str(s).expect("valid JSON")
    }

    const PAGES: &str = r#"{"event":"pages","url":"https://example.test/auto","manual_url":"https://example.test/manual","manual_first":false}"#;

    /// A stand-in for the CLI: a script the system's own shell runs.
    #[cfg(windows)]
    fn script(folder: &std::path::Path, lines: &[&str]) -> Command {
        let path = folder.join("fake.cmd");
        std::fs::write(&path, format!("@echo off\r\n{}\r\n", lines.join("\r\n"))).expect("the script");
        let mut cmd = Command::new("cmd");
        cmd.arg("/C").arg(path);
        cmd
    }
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "freebsd"))]
    fn script(folder: &std::path::Path, lines: &[&str]) -> Command {
        let path = folder.join("fake.sh");
        std::fs::write(&path, format!("{}\n", lines.join("\n"))).expect("the script");
        let mut cmd = Command::new("sh");
        cmd.arg(path);
        cmd
    }

    /// Names its pages, then waits for a line: a code ends it well, no line ends it badly.
    #[cfg(windows)]
    fn names_pages_then_wants_a_code(folder: &std::path::Path) -> Command {
        script(folder, &[
            &format!("echo {PAGES}"),
            "set \"line=\"",
            "set /p line=",
            r#"if defined line (echo {"event":"done","ok":true}) else (echo {"event":"done","ok":false,"message":"no code came"})"#,
        ])
    }
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "freebsd"))]
    fn names_pages_then_wants_a_code(folder: &std::path::Path) -> Command {
        script(folder, &[
            &format!("echo '{PAGES}'"),
            r#"if read line; then echo '{"event":"done","ok":true}'; else echo '{"event":"done","ok":false,"message":"no code came"}'; fi"#,
        ])
    }

    /// Says `complaint` on stderr, if anything, and leaves with exit code 3.
    #[cfg(windows)]
    fn leaves_at_once(folder: &std::path::Path, complaint: &str) -> Command {
        let say = if complaint.is_empty() { "rem".to_string() } else { format!("echo {complaint} 1>&2") };
        script(folder, &[&say, "exit /b 3"])
    }
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "freebsd"))]
    fn leaves_at_once(folder: &std::path::Path, complaint: &str) -> Command {
        let say = if complaint.is_empty() { ":".to_string() } else { format!("echo '{complaint}' 1>&2") };
        script(folder, &[&say, "exit 3"])
    }

    fn gone_within(run: &SignIn, limit: Duration) -> bool {
        let since = Instant::now();
        while run.exited().is_none() && since.elapsed() < limit {
            std::thread::sleep(Duration::from_millis(25));
        }
        run.exited().is_some()
    }

    #[test]
    fn the_pages_and_the_ending_are_read_off_the_clis_lines() {
        assert_eq!(
            event(PAGES),
            Some(Event::Pages { url: "https://example.test/auto".into(), manual_url: "https://example.test/manual".into(), manual_first: false })
        );
        let manual = r#"{"event":"pages","url":"u","manual_url":"m","manual_first":true}"#;
        assert_eq!(event(manual), Some(Event::Pages { url: "u".into(), manual_url: "m".into(), manual_first: true }));
        assert_eq!(event(r#"{"event":"done","ok":true}"#), Some(Event::Done { ok: true, message: None }));
        assert_eq!(
            event("{\"event\":\"done\",\"ok\":false,\"message\":\"Denied\"}\r"),
            Some(Event::Done { ok: false, message: Some("Denied".into()) })
        );
    }

    #[test]
    fn a_line_that_is_not_an_event_is_passed_over() {
        for line in [
            "", "not json", "[]", r#"{"event":"progress"}"#, r#"{"event":"pages","url":"u"}"#,
            r#"{"event":"done"}"#, r#"{"event":"done","ok":"yes"}"#, r#"{"ok":true}"#,
        ] {
            assert_eq!(event(line), None, "{line}");
        }
    }

    #[test]
    fn a_pasted_code_goes_in_whole_as_one_line() {
        assert_eq!(parsed(&code_line("  abc#def \n").unwrap()), json!({"code":"abc#def"}));
        assert!(!code_line("a\"b#c").unwrap().contains('\n'));
        assert_eq!(parsed(&code_line("a\"b#c").unwrap()), json!({"code":"a\"b#c"}));
    }

    #[test]
    fn what_is_not_a_code_is_refused_with_the_reason() {
        assert_eq!(code_line("  ").unwrap_err(), "That is not an authorization code.");
        assert_eq!(code_line(&format!("{}#x", "a".repeat(4096))).unwrap_err(), "That is not an authorization code.");
        assert_eq!(code_line("abcdef").unwrap_err(), "Paste the whole code from the page, including the part after the #.");
    }

    #[test]
    fn a_process_that_goes_without_a_word_is_described_by_how_it_went() {
        assert_eq!(ended_message("", Some(3)), "The sign-in ended before it finished (exit code 3). Try again.");
        assert_eq!(ended_message(" \n", None), "The sign-in ended before it finished (stopped). Try again.");
        assert_eq!(ended_message("  Not signed in to claude.ai\n", Some(1)), "Not signed in to claude.ai");
    }

    #[test]
    fn the_cli_is_started_for_a_conversation_in_json_lines() {
        assert_eq!(args(), ["design-login", "--json"]);
    }

    #[test]
    fn the_answers_say_what_the_dialog_needs() {
        let pages = Event::Pages { url: "u".into(), manual_url: "m".into(), manual_first: true };
        assert_eq!(parsed(&start_json(Ok(pages))), json!({"ok":true,"url":"u","manualUrl":"m","manualFirst":true}));
        assert_eq!(parsed(&start_json(Err(Some("Denied".into())))), json!({"ok":false,"message":"Denied"}));
        assert_eq!(parsed(&start_json(Err(None))), json!({"ok":false,"message":COULD_NOT_START}));
        assert_eq!(parsed(&outcome_json((true, None))), json!({"ok":true}));
        assert_eq!(parsed(&outcome_json((false, Some("The sign-in timed out.".into())))), json!({"ok":false,"message":"The sign-in timed out."}));
    }

    #[test]
    fn a_step_that_is_not_one_and_a_code_nobody_waits_for_are_refused() {
        assert_eq!(parsed(&run("claude", "", "status", "")), json!({"ok":false,"message":"Invalid request."}));
        assert_eq!(parsed(&run("claude", "", "code", "abcdef")), json!({"ok":false,"message":"Paste the whole code from the page, including the part after the #."}));
    }

    #[test]
    fn a_sign_in_names_its_pages_takes_a_code_and_ends_well() {
        let folder = tempfile::tempdir().expect("a folder");
        let run = drive(names_pages_then_wants_a_code(folder.path()), Duration::from_secs(20)).expect("started");
        assert_eq!(
            run.started(),
            Ok(Event::Pages { url: "https://example.test/auto".into(), manual_url: "https://example.test/manual".into(), manual_first: false })
        );
        assert!(run.say(&code_line("abc#def").unwrap()), "the code went in");
        assert_eq!(run.outcome(), (true, None));
        assert!(gone_within(&run, Duration::from_secs(10)), "its process is seen out");
    }

    #[test]
    fn cancelling_ends_it_at_once_and_its_process_goes() {
        let folder = tempfile::tempdir().expect("a folder");
        let run = drive(names_pages_then_wants_a_code(folder.path()), Duration::from_secs(20)).expect("started");
        assert!(run.started().is_ok());
        run.finish(false, None);
        assert_eq!(run.outcome(), (false, None), "what the script says afterwards changes nothing");
        assert!(gone_within(&run, Duration::from_secs(10)), "its process is seen out");
        assert!(!run.say("{}"), "and nothing more can be said to it");
    }

    #[test]
    fn a_sign_in_nobody_finishes_times_out() {
        let folder = tempfile::tempdir().expect("a folder");
        // Whether its pages were named before the limit is the machine's speed, not the point.
        let run = drive(names_pages_then_wants_a_code(folder.path()), Duration::from_millis(400)).expect("started");
        assert_eq!(run.outcome(), (false, Some("The sign-in timed out.".into())));
        assert!(gone_within(&run, Duration::from_secs(10)), "its process is seen out");
    }

    #[test]
    fn a_cli_that_leaves_before_naming_its_pages_says_why() {
        let folder = tempfile::tempdir().expect("a folder");
        let run = drive(leaves_at_once(folder.path(), "boom"), Duration::from_secs(20)).expect("started");
        assert_eq!(run.started(), Err(Some("boom".into())));
        let quiet = tempfile::tempdir().expect("a folder");
        let run = drive(leaves_at_once(quiet.path(), ""), Duration::from_secs(20)).expect("started");
        assert_eq!(run.started(), Err(Some("The sign-in ended before it finished (exit code 3). Try again.".into())));
    }

    #[test]
    fn a_program_that_is_not_there_cannot_be_started() {
        assert!(drive(Command::new("claude-eclipse-no-such-program"), Duration::from_secs(1)).is_err());
    }
}
