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
