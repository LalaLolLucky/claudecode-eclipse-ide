//! Cross-platform spawning of the `claude` CLI.
//!
//! On macOS/Linux `Command::new(claude_cmd)` already does the right thing:
//! the kernel does PATH lookup for a bare name, and Rust's default argv
//! escaping matches what the child's C runtime un-escapes.
//!
//! Windows needs help on two fronts, and BOTH bit real users (issues #64/#83):
//!
//!   1. **Bare name resolution.** `claude` installed via `npm -g` is
//!      `claude.cmd` on `%PATH%`. `Command::new("claude")` calls
//!      `CreateProcess("claude")`, which does NOT consult `PATHEXT`, so it
//!      fails with `CreateProcess error=2` even though `where claude` works in
//!      a shell. We resolve the name against PATH + PATHEXT ourselves.
//!
//!   2. **`.cmd`/`.bat` wrappers.** A batch file can't be executed by
//!      `CreateProcess` directly; it runs through `cmd.exe /c`, which parses the
//!      line, and parses it again wherever the wrapper forwards `%*`.
//!
//! npm's own `claude.cmd` only forwards to a `claude.exe`, so that exe is started
//! directly. Any other batch file — a hand-written wrapper, or npm's shim with lines
//! added — runs through `cmd.exe` with every argument quoted to survive both passes
//! (`quote_batch_arg`). `--mcp-config` still goes by temp file on Windows
//! (`chat::mcp_config_value`): that fix predates this quoting and stays.

use std::process::Command;

/// The default command used when the Claude-command preference is left blank.
/// Matches the Java-side `Constants.DEFAULT_CLAUDE_CMD`; on every platform we
/// resolve it against PATH before spawning.
const DEFAULT_CLAUDE_CMD: &str = "claude";

/// Builds a `Command` for the `claude` CLI with `args`, handling Windows
/// PATH/PATHEXT resolution and `.cmd`/`.bat` quoting. The caller still sets
/// `current_dir`, stdio, env vars and (Windows) `creation_flags` — this only
/// owns the program + argument wiring so JSON args survive on every platform.
///
/// `claude_cmd` may be an absolute path, a bare name (`claude`), or empty
/// (→ the default `claude`, resolved against PATH like macOS/Linux do).
pub fn claude_command(claude_cmd: &str, args: &[String]) -> Command {
    let requested = if claude_cmd.trim().is_empty() {
        DEFAULT_CLAUDE_CMD
    } else {
        claude_cmd.trim()
    };

    #[cfg(windows)]
    {
        build_windows(requested, args)
    }
    #[cfg(not(windows))]
    {
        // The kernel resolves a bare name via PATH; absolute paths pass through.
        let mut cmd = Command::new(requested);
        cmd.args(args);
        cmd
    }
}

/// The `command`/`args` of an MCP **stdio** server config that runs the `claude`
/// CLI with `args`, for a server the CLI process launches itself (the
/// `claude-in-chrome` server added over `mcp_set_servers`). On Windows the name is
/// resolved against PATH/PATHEXT first, and an npm `.cmd` shim is followed to the
/// `claude.exe` it forwards to, so no batch file is left for the CLI to launch; a
/// shim that can't be read runs through `cmd.exe /d /c` instead.
pub fn stdio_server_command(claude_cmd: &str, args: &[&str]) -> (String, Vec<String>) {
    let requested = if claude_cmd.trim().is_empty() {
        DEFAULT_CLAUDE_CMD
    } else {
        claude_cmd.trim()
    };
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();

    #[cfg(windows)]
    {
        let resolved = resolve_windows(requested);
        if !is_batch(&resolved) {
            return (resolved, args);
        }
        if let Some(exe) = npm_shim_target(&resolved) {
            return (exe, args);
        }
        let comspec = std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".to_string());
        let mut cmd_args = vec!["/d".to_string(), "/c".to_string(), resolved];
        cmd_args.extend(args);
        (comspec, cmd_args)
    }
    #[cfg(not(windows))]
    {
        (requested.to_string(), args)
    }
}

/// The names to look for the CLI's own program under, in order: what the user
/// configured, then the plain default. The second is what saves the lookup when the
/// first is a wrapper we cannot read through — a hand-written `.bat`, or a shell
/// script — since a wrapper almost always sits in front of an ordinary install.
///
/// This decides only which program is READ (the bundled instruction text, the flag
/// list). What actually runs is always the configured command, wrapper and all.
fn program_candidates(claude_cmd: &str) -> Vec<String> {
    let requested = requested_command(claude_cmd).to_string();
    if requested == DEFAULT_CLAUDE_CMD {
        return vec![requested];
    }
    vec![requested, DEFAULT_CLAUDE_CMD.to_string()]
}

/// A candidate only counts when it is the CLI's own program: a file, and not a `#!`
/// script. npm installs and hand-written wrappers put a script on PATH in front of
/// the binary, and the text we read lives in the binary — so a script is passed over
/// here exactly as a non-npm `.bat` is, and the next candidate gets its turn.
fn usable_program(path: &std::path::Path) -> Option<std::path::PathBuf> {
    if !path.is_file() {
        return None;
    }
    let mut head = [0u8; 2];
    if let Ok(mut file) = std::fs::File::open(path) {
        use std::io::Read;
        if file.read_exact(&mut head).is_ok() && &head == b"#!" {
            return wrapped_program(path);
        }
    }
    Some(path.to_path_buf())
}

/// The program behind a `#!` wrapper, where the install layout says which. On FreeBSD,
/// claude-freebsd puts a sh wrapper at `<prefix>/bin/claude` that execs the Linux build
/// it installs at `<prefix>/libexec/claude-code/claude`.
#[cfg(target_os = "freebsd")]
fn wrapped_program(wrapper: &std::path::Path) -> Option<std::path::PathBuf> {
    let program = wrapper.parent()?.parent()?.join("libexec/claude-code/claude");
    program.is_file().then_some(program)
}

/// Any other wrapper is someone's own, and the program it runs cannot be known.
#[cfg(not(target_os = "freebsd"))]
fn wrapped_program(_wrapper: &std::path::Path) -> Option<std::path::PathBuf> {
    None
}

/// The file the `claude` command runs — the one to read the CLI's own bundled text
/// out of (see `chrome::cli_instruction`). Each candidate is resolved against
/// PATH/PATHEXT and an npm `.cmd` shim is followed to its `claude.exe`; any other
/// batch file is someone's own wrapper, which stops that candidate and moves to the
/// next. `None` when none of them lands on a readable program.
#[cfg(windows)]
pub fn claude_program_file(claude_cmd: &str) -> Option<std::path::PathBuf> {
    program_candidates(claude_cmd).into_iter().find_map(|name| {
        let resolved = resolve_windows(&name);
        let file = if is_batch(&resolved) { shim_forward_target(&resolved)? } else { resolved };
        usable_program(std::path::Path::new(&file))
    })
}

/// The file the `claude` command runs: a path as given, or a bare name found on PATH
/// ([`program_candidates`] in turn). Symlinks (the native installer's
/// `~/.local/bin/claude`) are followed when read; a `#!` wrapper is passed over.
#[cfg(target_os = "macos")]
pub fn claude_program_file(claude_cmd: &str) -> Option<std::path::PathBuf> {
    program_candidates(claude_cmd).into_iter().find_map(|name| find_on_path(&name))
}

/// The file the `claude` command runs: a path as given, or a bare name found on PATH
/// ([`program_candidates`] in turn). Symlinks (the native installer's
/// `~/.local/bin/claude`) are followed when read; a `#!` wrapper is passed over.
#[cfg(target_os = "linux")]
pub fn claude_program_file(claude_cmd: &str) -> Option<std::path::PathBuf> {
    program_candidates(claude_cmd).into_iter().find_map(|name| find_on_path(&name))
}

/// The file the `claude` command runs: a path as given, or a bare name found on PATH
/// ([`program_candidates`] in turn). Symlinks (the native installer's
/// `~/.local/bin/claude`) are followed when read; claude-freebsd's `#!` wrapper is
/// followed to the binary it execs ([`wrapped_program`]), and any other is passed over.
#[cfg(target_os = "freebsd")]
pub fn claude_program_file(claude_cmd: &str) -> Option<std::path::PathBuf> {
    program_candidates(claude_cmd).into_iter().find_map(|name| find_on_path(&name))
}

/// Whether the installed CLI knows `flag`, by looking for it in the program itself.
///
/// A flag the CLI does not know aborts it at startup ("unknown option"), taking the
/// chat with it, so anything version-dependent has to be checked before it is passed
/// — the same reason the GUI gates `--thinking-display`. Answers false when the
/// program cannot be found or read, which keeps the older behaviour rather than
/// risking that abort. Cached per program file (path, size, mtime): the first call
/// reads through the binary, later ones only stat it.
pub fn cli_supports_flag(claude_cmd: &str, flag: &str) -> bool {
    type Key = (std::path::PathBuf, u64, Option<std::time::SystemTime>, String);
    static CACHE: std::sync::Mutex<Vec<(Key, bool)>> = std::sync::Mutex::new(Vec::new());
    const SCAN_CHUNK: usize = 8 * 1024 * 1024;

    let Some(path) = claude_program_file(claude_cmd) else { return false };
    let Ok(meta) = std::fs::metadata(&path) else { return false };
    let key: Key = (path, meta.len(), meta.modified().ok(), flag.to_string());

    let mut cache = CACHE.lock().unwrap_or_else(|p| p.into_inner());
    if let Some((_, found)) = cache.iter().find(|(k, _)| *k == key) {
        return *found;
    }
    let found = std::fs::File::open(&key.0)
        .ok()
        .and_then(|mut f| crate::chrome::find_in_reader(&mut f, flag.as_bytes(), SCAN_CHUNK))
        .is_some();
    // One CLI at a time in practice; a handful of entries is the whole cache.
    if cache.len() > 8 {
        cache.clear();
    }
    cache.push((key, found));
    found
}

/// Ends a process started with [`claude_command`] together with everything it
/// started. On Windows a batch wrapper (a hand-written `claude.bat`) still runs
/// through `cmd.exe`, and killing only that leaves `claude.exe` behind, so the whole
/// tree goes with `taskkill /T`.
#[cfg(windows)]
pub fn kill_process_tree(child: &mut std::process::Child) {
    use std::os::windows::process::CommandExt;
    // While the parent is alive, so taskkill can still find its children.
    let _ = Command::new("taskkill")
        .args(["/PID", &child.id().to_string(), "/T", "/F"])
        .creation_flags(0x08000000) // CREATE_NO_WINDOW
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    let _ = child.kill();
}

/// Ends a process started with [`claude_command`]. `claude` is started directly here,
/// with no wrapper in between, so the process itself is the one to end.
#[cfg(target_os = "macos")]
pub fn kill_process_tree(child: &mut std::process::Child) {
    let _ = child.kill();
}

/// Ends a process started with [`claude_command`]. `claude` is started directly here,
/// with no wrapper in between, so the process itself is the one to end.
#[cfg(target_os = "linux")]
pub fn kill_process_tree(child: &mut std::process::Child) {
    let _ = child.kill();
}

/// Ends a process started with [`claude_command`]. `claude` is started directly here,
/// with no wrapper in between, so the process itself is the one to end.
#[cfg(target_os = "freebsd")]
pub fn kill_process_tree(child: &mut std::process::Child) {
    let _ = child.kill();
}

fn requested_command(claude_cmd: &str) -> &str {
    if claude_cmd.trim().is_empty() {
        DEFAULT_CLAUDE_CMD
    } else {
        claude_cmd.trim()
    }
}

/// `name` itself when it is a path, else the first file of that name in a PATH directory.
#[cfg_attr(windows, allow(dead_code))]
/// The program a name resolves to: an absolute or relative path as given, or a bare
/// name searched on the LOGIN SHELL's PATH first and Eclipse's own PATH second.
///
/// The shell's PATH comes first because Eclipse started from Finder or a desktop
/// entry inherits almost nothing, which is why every Claude process is spawned with
/// the captured shell PATH injected (`shell_env::to_inject`). Searching only the
/// process PATH here left us unable to FIND the program we can perfectly well RUN.
fn find_on_path(name: &str) -> Option<std::path::PathBuf> {
    if name.contains('/') {
        return usable_program(std::path::Path::new(name));
    }
    let shell_path = crate::shell_env::captured_env().path.clone();
    let process_path = std::env::var("PATH").ok();
    shell_path
        .iter()
        .chain(process_path.iter())
        .flat_map(|paths| std::env::split_paths(paths).collect::<Vec<_>>())
        .find_map(|dir| usable_program(&dir.join(name)))
}

// ───────────────────────────────────────────────────────────────────────────
// Windows
// ───────────────────────────────────────────────────────────────────────────

#[cfg(windows)]
fn is_batch(path: &str) -> bool {
    path.rsplit('.')
        .next()
        .map(|ext| ext.eq_ignore_ascii_case("cmd") || ext.eq_ignore_ascii_case("bat"))
        .unwrap_or(false)
}

/// The executable an npm-generated `.cmd` shim forwards to — the quoted
/// `"%dp0%\…\claude.exe"` on its last line — when the file is exactly that shim
/// and the exe exists. A copy someone has added to (a `SET HTTPS_PROXY=…`, say) is
/// their wrapper, not npm's: going straight to the exe would drop what they added,
/// so it runs through cmd.exe like any other wrapper.
#[cfg(windows)]
fn npm_shim_target(shim: &str) -> Option<String> {
    // Every line npm's cmd-shim writes before the forwarding one, in order.
    const PREAMBLE: [&str; 8] = [
        "@echo off", "goto start", ":find_dp0", "set dp0=%~dp0",
        "exit /b", ":start", "setlocal", "call :find_dp0",
    ];
    const DP0: &str = "\"%dp0%\\";
    let text = std::fs::read_to_string(shim).ok()?;
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let (last, preamble) = lines.split_last()?;
    if preamble.len() != PREAMBLE.len()
        || !preamble.iter().zip(PREAMBLE).all(|(l, p)| l.eq_ignore_ascii_case(p))
    {
        return None;
    }
    let (rel, tail) = last.strip_prefix(DP0)?.split_once('"')?;
    if tail.trim() != "%*" || !rel.to_ascii_lowercase().ends_with(".exe") {
        return None;
    }
    let exe = std::path::Path::new(shim).parent()?.join(rel);
    exe.is_file().then(|| exe.to_string_lossy().into_owned())
}

/// The exe a shim forwards to (`"%dp0%\…\claude.exe"`), whatever else the file
/// holds. Only for READING the CLI ([`claude_program_file`]): a line someone added
/// to npm's shim changes how it runs, not which program it is — so it is still the
/// file to read, where [`npm_shim_target`] would refuse to run it.
#[cfg(windows)]
fn shim_forward_target(shim: &str) -> Option<String> {
    const DP0: &str = "\"%dp0%\\";
    let text = std::fs::read_to_string(shim).ok()?;
    let start = text.find(DP0)? + DP0.len();
    let rel = &text[start..start + text[start..].find('"')?];
    if !rel.to_ascii_lowercase().ends_with(".exe") {
        return None;
    }
    let exe = std::path::Path::new(shim).parent()?.join(rel);
    exe.is_file().then(|| exe.to_string_lossy().into_owned())
}

#[cfg(windows)]
fn build_windows(requested: &str, args: &[String]) -> Command {
    use std::os::windows::process::CommandExt;

    // Resolve to a concrete file so we can (a) find `claude.cmd` for a bare
    // name and (b) know whether it's a batch script or a real executable.
    let resolved = resolve_windows(requested);

    if !is_batch(&resolved) {
        // Real .exe (or unresolved bare name we couldn't map — let CreateProcess
        // try, same as before). Rust's default argv escaping is correct here.
        let mut cmd = Command::new(&resolved);
        cmd.args(args);
        return cmd;
    }

    // An npm install's `claude.cmd` only forwards to a `claude.exe`: start that exe
    // itself. Through the shim the process we hold is `cmd.exe`, and killing it leaves
    // `claude.exe` running with no parent, still holding its conversation and its
    // Remote Control link. An exe also takes Rust's own argv quoting as-is, so the
    // JSON args need none of the cmd.exe handling below.
    if let Some(exe) = npm_shim_target(&resolved) {
        let mut cmd = Command::new(exe);
        cmd.args(args);
        return cmd;
    }

    // Any other batch file is someone's wrapper, and the only way to run one is
    // cmd.exe — which parses the line once here, and again wherever the wrapper
    // forwards `%*`. Every argument is written to survive both passes
    // (quote_batch_arg), and the whole command sits inside one more pair of
    // quotes: `cmd /c` strips the first and last `"` of a line that starts with
    // one, which otherwise ate the quotes round a wrapper path with a space in it.
    // /d: no AutoRun commands. /e:on: the `%` escape needs command extensions.
    // /v:off: `!` stays literal.
    let comspec = std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".to_string());
    let mut line = String::from("/d /e:on /v:off /c \"");
    line.push_str(&quote_batch_arg(&resolved));
    for arg in args {
        line.push(' ');
        line.push_str(&quote_batch_arg(arg));
    }
    line.push('"');

    let mut cmd = Command::new(comspec);
    // raw_arg bypasses Rust's per-arg escaping entirely — we hand cmd.exe the
    // exact command line, already correctly quoted.
    cmd.raw_arg(&line);
    cmd
}

/// Resolves a Windows command name to a concrete path. Absolute/relative paths
/// with an extension are returned as-is; a bare name is searched on `%PATH%`
/// with each `%PATHEXT%` suffix (so `claude` → `...\claude.cmd`). Returns the
/// input unchanged when nothing matches (CreateProcess then reports the error,
/// preserving the old behavior).
#[cfg(windows)]
fn resolve_windows(requested: &str) -> String {
    use std::path::Path;

    let p = Path::new(requested);

    // Already a path (contains a separator) AND already has an extension → use
    // it directly. A path without an extension still needs PATHEXT probing.
    let has_sep = requested.contains('\\') || requested.contains('/');
    if has_sep && p.extension().is_some() && p.is_file() {
        return requested.to_string();
    }

    let pathext: Vec<String> = std::env::var("PATHEXT")
        .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string())
        .split(';')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();

    // If the name already carries a known extension, don't append another.
    let already_has_ext = p
        .extension()
        .map(|e| {
            let dotted = format!(".{}", e.to_string_lossy());
            pathext.iter().any(|x| x.eq_ignore_ascii_case(&dotted))
        })
        .unwrap_or(false);

    // A path with a separator: probe extensions next to it, don't walk PATH.
    if has_sep {
        if already_has_ext && p.is_file() {
            return requested.to_string();
        }
        for ext in &pathext {
            let candidate = format!("{}{}", requested, ext);
            if Path::new(&candidate).is_file() {
                return candidate;
            }
        }
        return requested.to_string();
    }

    // Bare name: walk %PATH%, probing PATHEXT in each directory.
    let path_var = std::env::var("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&path_var) {
        if already_has_ext {
            let candidate = dir.join(requested);
            if candidate.is_file() {
                return candidate.to_string_lossy().into_owned();
            }
        }
        for ext in &pathext {
            let candidate = dir.join(format!("{}{}", requested, ext));
            if candidate.is_file() {
                return candidate.to_string_lossy().into_owned();
            }
        }
    }

    requested.to_string()
}

/// Quotes one argument (or the wrapper's own path) for a `cmd.exe /c` line that
/// runs a batch wrapper, as Rust's standard library does for batch files:
///
/// * always inside `"…"`, so cmd.exe's `& | < > ^ ( )` are inert in every pass;
/// * an inner `"` doubled to `""`, which keeps cmd.exe's quote-tracking in step and
///   which `claude.exe` reads back as one `"` (checked against 2.1.280, 2026-09-28);
/// * the run of backslashes before a `"` or the closing quote doubled, as MSVCRT
///   parsing expects;
/// * each `%` written `%%cd:~,%`, which cmd.exe turns back into a bare `%` rather
///   than expanding `%NAME%`.
///
/// A line break cannot be carried through cmd.exe at all. No caller passes one:
/// the MCP window splits its fields into lines first.
#[cfg(windows)]
fn quote_batch_arg(arg: &str) -> String {
    let mut out = String::with_capacity(arg.len() + 2);
    out.push('"');
    let mut backslashes = 0usize;
    for c in arg.chars() {
        if c == '\\' {
            backslashes += 1;
            out.push(c);
            continue;
        }
        if c == '"' {
            out.extend(std::iter::repeat('\\').take(backslashes));
            out.push('"');
        } else if c == '%' {
            out.push_str("%%cd:~,");
        }
        backslashes = 0;
        out.push(c);
    }
    out.extend(std::iter::repeat('\\').take(backslashes));
    out.push('"');
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn batch_args_are_always_quoted() {
        assert_eq!(quote_batch_arg("--verbose"), r#""--verbose""#);
        assert_eq!(quote_batch_arg("https://h/mcp?a=1&b=2"), r#""https://h/mcp?a=1&b=2""#);
        assert_eq!(quote_batch_arg(""), r#""""#);
    }

    #[test]
    fn batch_arg_quotes_double_and_percent_is_broken_up() {
        assert_eq!(quote_batch_arg(r#"say "hi""#), r#""say ""hi""""#);
        assert_eq!(quote_batch_arg("%PATH%"), r#""%%cd:~,%PATH%%cd:~,%""#);
    }

    #[test]
    fn batch_arg_backslashes_doubled_only_before_a_quote() {
        assert_eq!(quote_batch_arg(r"C:\x y\"), r#""C:\x y\\""#);
        assert_eq!(quote_batch_arg(r#"a\"b"#), r#""a\\""b""#);
        assert_eq!(quote_batch_arg(r"a\b"), r#""a\b""#);
    }

    #[test]
    fn a_wrapper_line_is_quoted_whole() {
        let cmd = claude_command(r"C:\nowhere-claude-eclipse\my wrapper.cmd", &["a&b".to_string()]);
        let line: Vec<_> = cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(line, [r#"/d /e:on /v:off /c ""C:\nowhere-claude-eclipse\my wrapper.cmd" "a&b"""#]);
    }

    #[test]
    fn an_added_to_npm_shim_is_a_wrapper() {
        // npm's shim with one line of the user's own: going straight to the exe would
        // drop that line, so it has to run through cmd.exe.
        let dir = std::env::temp_dir().join("claude-eclipse-edited shim");
        let _ = std::fs::remove_dir_all(&dir);
        let exe = dir.join(r"node_modules\@anthropic-ai\claude-code\bin\claude.exe");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, "").unwrap();
        let shim = dir.join("claude.cmd");
        std::fs::write(&shim, "@ECHO off\r\nGOTO start\r\n:find_dp0\r\nSET dp0=%~dp0\r\nEXIT /b\r\n:start\r\nSETLOCAL\r\nCALL :find_dp0\r\nSET HTTPS_PROXY=http://proxy:8080\r\n\"%dp0%\\node_modules\\@anthropic-ai\\claude-code\\bin\\claude.exe\"   %*\r\n").unwrap();

        let target = npm_shim_target(&shim.to_string_lossy());
        let cmd = claude_command(&shim.to_string_lossy(), &["--x".to_string()]);
        let read = claude_program_file(&shim.to_string_lossy());
        let _ = std::fs::remove_dir_all(&dir);

        assert!(target.is_none());
        assert!(cmd.get_program().to_string_lossy().to_ascii_lowercase().ends_with("cmd.exe"));
        // Still the program to READ (flag support, bundled text): the added line
        // changes how it runs, not what it is.
        assert_eq!(read.as_deref(), Some(exe.as_path()));
    }

    /// Every form the Claude-command preference can take reaches the installed CLI:
    /// blank, the bare name, `claude.cmd`, the shim's full path, the exe's full path,
    /// and a hand-written wrapper in a folder with a space. The npm forms must start
    /// `claude.exe` itself (no cmd.exe in between). Needs an npm install of Claude
    /// Code on PATH, so it only runs when asked: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn every_command_form_reaches_the_installed_cli() {
        let shim = resolve_windows("claude");
        let Some(exe) = npm_shim_target(&shim) else { return };   // not an npm install
        let dir = std::env::temp_dir().join("claude-eclipse my wrapper");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let wrapper = dir.join("claude-wrap.cmd");
        std::fs::write(&wrapper, format!("@echo off\r\nsetlocal\r\nset CLAUDE_ECLIPSE_WRAPPED=1\r\n\"{shim}\" %*\r\n")).unwrap();

        let forms = ["", "claude", "claude.cmd", shim.as_str(), exe.as_str(), &wrapper.to_string_lossy()];
        let mut failures = Vec::new();
        for (i, form) in forms.iter().enumerate() {
            let mut cmd = claude_command(form, &["--version".to_string()]);
            let direct = std::path::Path::new(cmd.get_program()) == std::path::Path::new(&exe);
            if i < 5 && !direct {
                failures.push(format!("{form:?}: not started as claude.exe ({:?})", cmd.get_program()));
            }
            match cmd.output() {
                Ok(o) if String::from_utf8_lossy(&o.stdout).contains("(Claude Code)") => {}
                Ok(o) => failures.push(format!("{form:?}: {} / {}", String::from_utf8_lossy(&o.stdout).trim(),
                                               String::from_utf8_lossy(&o.stderr).trim())),
                Err(e) => failures.push(format!("{form:?}: {e}")),
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
        assert!(failures.is_empty(), "{failures:#?}");
    }

    /// End to end through a real wrapper that forwards `%*`, in a folder whose name
    /// has a space, a `%` and a `&`. Needs `python` on PATH to print what arrives,
    /// so it only runs when asked: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn a_wrapper_receives_every_argument_exactly() {
        let python = match std::process::Command::new("where").arg("python").output() {
            Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).lines().next().unwrap_or("").trim().to_string(),
            _ => return,
        };
        let dir = std::env::temp_dir().join("claude-eclipse wrap 100%&co");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("argv.py"), "import sys, json\nfor a in sys.argv[1:]:\n    print(json.dumps(a))\n").unwrap();
        let wrapper = dir.join("claude wrap.cmd");
        std::fs::write(&wrapper, format!("@echo off\r\nsetlocal\r\n\"{python}\" \"%~dp0argv.py\" %*\r\n")).unwrap();

        let inputs: Vec<String> = [
            r#"{"mcpServers":{"x":{"type":"http","url":"https://h/?a=1&b=2"}}}"#,
            "https://api.example.com/mcp?team=a&key=%USERNAME%",
            r#"say "hi" & echo PWNED"#,
            "a|b<c>d^e (x)",
            "x!y!z",
            "100%",
            r#"a\"b"#,
            r"C:\x y\",
            "",
            "Authorization: Bearer tok&%PATH%",
            "caf\u{e9} \u{2014} dash",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();

        let out = claude_command(&wrapper.to_string_lossy(), &inputs)
            .env("PYTHONIOENCODING", "utf-8")
            .output()
            .unwrap();
        let _ = std::fs::remove_dir_all(&dir);

        let got: Vec<String> = String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(got, inputs, "stderr: {}", String::from_utf8_lossy(&out.stderr));
    }

    #[test]
    fn stdio_server_follows_npm_shim_to_exe() {
        let dir = std::env::temp_dir().join("claude-eclipse-npm shim");
        let _ = std::fs::remove_dir_all(&dir);
        let exe = dir.join(r"node_modules\@anthropic-ai\claude-code\bin\claude.exe");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, "").unwrap();
        let shim = dir.join("claude.cmd");
        // The shim npm generates, verbatim.
        std::fs::write(&shim, "@ECHO off\r\nGOTO start\r\n:find_dp0\r\nSET dp0=%~dp0\r\nEXIT /b\r\n:start\r\nSETLOCAL\r\nCALL :find_dp0\r\n\"%dp0%\\node_modules\\@anthropic-ai\\claude-code\\bin\\claude.exe\"   %*\r\n").unwrap();

        let (command, args) = stdio_server_command(&shim.to_string_lossy(), &["--claude-in-chrome-mcp"]);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(std::path::Path::new(&command), exe.as_path());
        assert_eq!(args, ["--claude-in-chrome-mcp"]);
    }

    #[test]
    fn stdio_server_unreadable_shim_runs_through_cmd() {
        let shim = r"C:\nowhere-claude-eclipse\claude.cmd";
        let (command, args) = stdio_server_command(shim, &["--x"]);
        assert!(command.to_ascii_lowercase().ends_with("cmd.exe"), "{command}");
        assert_eq!(args, ["/d", "/c", shim, "--x"]);
    }

    #[test]
    fn the_default_install_is_the_second_candidate() {
        // A wrapper is tried first and the plain default backs it up …
        assert_eq!(program_candidates(r"C:	oolsclaude.bat"), [r"C:	oolsclaude.bat", "claude"]);
        // … and nothing is tried twice when the two are the same.
        assert_eq!(program_candidates("claude"), ["claude"]);
        assert_eq!(program_candidates("   "), ["claude"]);
    }

    #[test]
    fn a_shebang_wrapper_is_not_the_program() {
        let dir = std::env::temp_dir().join("claude-eclipse-usable-program");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("claude-script");
        std::fs::write(&script, "#!/usr/bin/env node
require('./cli.js')
").unwrap();
        let binary = dir.join("claude-binary");
        std::fs::write(&binary, [0x4du8, 0x5a, 0x90, 0x00]).unwrap();   // an ordinary program
        let missing = dir.join("not-here");

        let script_result = usable_program(&script);
        let binary_result = usable_program(&binary);
        let missing_result = usable_program(&missing);
        let _ = std::fs::remove_dir_all(&dir);

        assert!(script_result.is_none(), "a #! wrapper carries none of the text we read");
        assert_eq!(binary_result.as_deref(), Some(binary.as_path()));
        assert!(missing_result.is_none());
    }

    #[test]
    fn a_custom_batch_never_resolves_to_itself() {
        // Someone's own .bat says nothing about the CLI's internals, so the lookup
        // moves on to the default install rather than returning the wrapper.
        let dir = std::env::temp_dir().join("claude-eclipse-custom-bat");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let bat = dir.join("claude.bat");
        std::fs::write(&bat, "@echo off
setlocal
node C:/tools/cli.js %*
").unwrap();

        let found = claude_program_file(&bat.to_string_lossy());
        let _ = std::fs::remove_dir_all(&dir);

        assert_ne!(found.as_deref(), Some(bat.as_path()));
        // Whatever it found (this machine has an npm install; a bare one has nothing)
        // is a real program, never a batch file.
        if let Some(path) = found {
            let ext = path.extension().unwrap_or_default().to_string_lossy().to_ascii_lowercase();
            assert!(ext != "bat" && ext != "cmd", "resolved to a wrapper: {}", path.display());
        }
    }
    #[test]
    fn claude_command_starts_the_exe_behind_an_npm_shim() {
        let dir = std::env::temp_dir().join("claude-eclipse-npm shim launch");
        let _ = std::fs::remove_dir_all(&dir);
        let exe = dir.join(r"node_modules\@anthropic-ai\claude-code\bin\claude.exe");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, "").unwrap();
        let shim = dir.join("claude.cmd");
        std::fs::write(&shim, "@ECHO off\r\nGOTO start\r\n:find_dp0\r\nSET dp0=%~dp0\r\nEXIT /b\r\n:start\r\nSETLOCAL\r\nCALL :find_dp0\r\n\"%dp0%\\node_modules\\@anthropic-ai\\claude-code\\bin\\claude.exe\"   %*\r\n").unwrap();
        let json = r#"{"mcpServers":{"eclipse":{"type":"sse"}}}"#.to_string();

        let cmd = claude_command(&shim.to_string_lossy(), &["--mcp-config".to_string(), json.clone()]);
        let unreadable = claude_command(r"C:\nowhere-claude-eclipse\claude.cmd", &["--x".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);

        // No cmd.exe in between, so killing the process ends Claude itself.
        assert_eq!(std::path::Path::new(cmd.get_program()), exe.as_path());
        let args: Vec<_> = cmd.get_args().collect();
        assert_eq!(args, ["--mcp-config", json.as_str()]);
        // A batch file that isn't npm's still goes through cmd.exe.
        assert!(unreadable.get_program().to_string_lossy().to_ascii_lowercase().ends_with("cmd.exe"));
    }
}
