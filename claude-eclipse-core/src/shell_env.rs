//! Login-shell environment capture for GUI-launched Eclipse.
//!
//! Eclipse launched from Finder/Dock (macOS) or a GNOME/KDE menu entry
//! (Linux) inherits a minimal process environment that does **not**
//! include anything the user set only in their shell rc files
//! (`~/.zshrc`, `~/.bashrc`, `~/.profile`, …).  Two practical consequences:
//!
//!   1. `claude` installed via nvm / asdf / Homebrew / `npm -g` prefix is
//!      invisible on PATH, so spawning it from Eclipse fails even though
//!      `which claude` works in the user's terminal.
//!   2. Corporate proxy variables (`HTTP_PROXY`, `HTTPS_PROXY`, `NO_PROXY`)
//!      set only in shell rc are silently missing, which makes Claude CLI
//!      fail with "Unable to connect" on locked-down networks.
//!
//! This module runs the user's login shell **once** (lazy, OnceLock-cached)
//! to capture PATH and the three proxy vars.  Callers inject whichever
//! values are present into child processes (chat + PTY).
//!
//! macOS and Linux have **fully independent** capture paths.  They do
//! similar work but are never unified under `cfg(unix)` or any other
//! umbrella — future divergence (different shell quirks, different
//! fallback logic) should not require refactoring, and keeping the
//! branches split makes it obvious at a glance what each platform does.

use std::sync::{Mutex, OnceLock};

/// Snapshot of the vars we care about from the user's login shell.
#[derive(Default, Clone, Debug)]
pub struct CapturedEnv {
    pub path:        Option<String>,
    pub http_proxy:  Option<String>,
    pub https_proxy: Option<String>,
    pub no_proxy:    Option<String>,
}

/// Proxy overrides set from Java preferences. Takes precedence over
/// process env and captured shell env.
#[derive(Default, Clone, Debug)]
pub struct ProxyOverrides {
    pub http_proxy:  Option<String>,
    pub https_proxy: Option<String>,
    pub no_proxy:    Option<String>,
}

static PROXY_OVERRIDES: OnceLock<Mutex<ProxyOverrides>> = OnceLock::new();

fn overrides() -> &'static Mutex<ProxyOverrides> {
    PROXY_OVERRIDES.get_or_init(|| Mutex::new(ProxyOverrides::default()))
}

/// Called from JNI when user changes proxy preferences.
pub fn set_proxy_overrides(http: Option<String>, https: Option<String>, no_proxy: Option<String>) {
    let mut guard = overrides().lock().unwrap();
    guard.http_proxy = http;
    guard.https_proxy = https;
    guard.no_proxy = no_proxy;
}

/// Returns current proxy overrides.
pub fn get_proxy_overrides() -> ProxyOverrides {
    overrides().lock().unwrap().clone()
}

impl CapturedEnv {
    /// Returns the `(key, value)` pairs to set on a spawned child process.
    ///
    /// Precedence (highest wins): preference override → process env → captured shell.
    ///
    /// - `PATH` is injected whenever captured — it overrides the sparse
    ///   inherited PATH, which is the whole point of capturing it.
    /// - Proxy vars follow the 3-tier precedence.
    /// - Auto-localhost safeguard: if any proxy is active and NO_PROXY doesn't
    ///   include localhost, we prepend it to prevent MCP traffic being routed
    ///   through a corporate proxy that can't see loopback.
    pub fn to_inject(&self) -> Vec<(&'static str, String)> {
        let mut out = Vec::new();
        let overrides = get_proxy_overrides();

        if let Some(ref p) = self.path {
            out.push(("PATH", p.clone()));
        }

        let http = resolve_proxy_var(
            &overrides.http_proxy,
            &["HTTP_PROXY", "http_proxy"],
            &self.http_proxy,
        );
        let https = resolve_proxy_var(
            &overrides.https_proxy,
            &["HTTPS_PROXY", "https_proxy"],
            &self.https_proxy,
        );
        let mut no_proxy = resolve_proxy_var(
            &overrides.no_proxy,
            &["NO_PROXY", "no_proxy"],
            &self.no_proxy,
        );

        // Auto-localhost safeguard: if any proxy is active and NO_PROXY
        // doesn't include localhost, prepend it.
        if (http.is_some() || https.is_some()) {
            no_proxy = Some(ensure_localhost_in_no_proxy(no_proxy));
        }

        if let Some(v) = http {
            out.push(("HTTP_PROXY", v));
        }
        if let Some(v) = https {
            out.push(("HTTPS_PROXY", v));
        }
        if let Some(v) = no_proxy {
            out.push(("NO_PROXY", v));
        }

        out
    }
}

/// Resolves a proxy var using 3-tier precedence:
/// 1. Preference override (if set and non-empty)
/// 2. Process environment (check both upper and lower case)
/// 3. Captured shell env (if set)
fn resolve_proxy_var(
    pref_override: &Option<String>,
    env_keys: &[&str],
    captured: &Option<String>,
) -> Option<String> {
    // 1. Preference override
    if let Some(ref v) = pref_override {
        if !v.is_empty() {
            return Some(v.clone());
        }
    }

    // 2. Process environment
    for key in env_keys {
        if let Ok(v) = std::env::var(key) {
            if !v.is_empty() {
                return Some(v);
            }
        }
    }

    // 3. Captured shell env
    captured.clone()
}

/// Ensures localhost/127.0.0.1/::1 are in NO_PROXY. An entry counts only when it
/// is one of them whole (ignoring case and surrounding spaces): a host such as
/// `notlocalhost.corp` merely contains `localhost` and does not keep loopback off
/// the proxy.
fn ensure_localhost_in_no_proxy(current: Option<String>) -> String {
    let localhost_entries = ["localhost", "127.0.0.1", "::1"];

    let present: Vec<String> = current.as_deref()
        .unwrap_or_default()
        .split(',')
        .map(|e| e.trim().to_lowercase())
        .collect();

    let mut missing: Vec<&str> = Vec::new();
    for entry in &localhost_entries {
        if !present.iter().any(|p| p == entry) {
            missing.push(entry);
        }
    }

    if missing.is_empty() {
        return current.unwrap_or_default();
    }

    let prefix = missing.join(",");
    match current {
        Some(v) if !v.is_empty() => format!("{},{}", prefix, v),
        _ => prefix,
    }
}

static CAPTURED: OnceLock<CapturedEnv> = OnceLock::new();

/// Returns the captured login-shell environment, computing it on first call
/// and caching for the life of the process.  Safe to call on all platforms.
pub fn captured_env() -> &'static CapturedEnv {
    CAPTURED.get_or_init(capture_impl)
}

// ───────────────────────────────────────────────────────────────────────────
// macOS
// ───────────────────────────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
fn capture_impl() -> CapturedEnv {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    // zsh is the macOS default since 10.15; honor $SHELL if the user changed it.
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());

    // `-l` login shell (sources .zprofile, /etc/paths*, etc.)
    // `-i` interactive (sources .zshrc, where most users set PATH/proxy)
    // The delimiter lets us skip banner/prompt noise and parse multi-value
    // output even when any value contains whitespace.
    const DELIM: &str = "__CEC_DELIM__";
    let script = format!(
        "printf '%s%s%s%s%s%s%s%s%s' \
           '{d}' \"$PATH\" \
           '{d}' \"${{HTTP_PROXY:-$http_proxy}}\" \
           '{d}' \"${{HTTPS_PROXY:-$https_proxy}}\" \
           '{d}' \"${{NO_PROXY:-$no_proxy}}\" \
           '{d}'",
        d = DELIM,
    );

    let mut child = match Command::new(&shell)
        .args(["-l", "-i", "-c", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c)  => c,
        Err(_) => return CapturedEnv::default(),
    };

    // 5s ceiling: a pathological .zshrc (blocks on network, waits for tty)
    // must not freeze the first chat/CLI spawn forever.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    return CapturedEnv::default();
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => return CapturedEnv::default(),
        }
    }

    let output = match child.wait_with_output() {
        Ok(o) if o.status.success() => o,
        _ => return CapturedEnv::default(),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Skip any banner noise before the first sentinel, then split the payload.
    let Some(start) = stdout.find(DELIM) else { return CapturedEnv::default(); };
    let parts: Vec<&str> = stdout[start + DELIM.len()..].split(DELIM).collect();
    let get = |i: usize| {
        parts.get(i)
             .map(|s| s.trim())
             .filter(|s| !s.is_empty())
             .map(|s| s.to_string())
    };

    CapturedEnv {
        path:        get(0),
        http_proxy:  get(1),
        https_proxy: get(2),
        no_proxy:    get(3),
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Linux
// ───────────────────────────────────────────────────────────────────────────

#[cfg(target_os = "linux")]
fn capture_impl() -> CapturedEnv {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    // bash is the Ubuntu/Debian default; honor $SHELL if the user changed it.
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());

    // `-l` sources /etc/profile + ~/.profile (or ~/.bash_profile).
    // `-i` sources ~/.bashrc, where Ubuntu users typically set PATH/proxy.
    const DELIM: &str = "__CEC_DELIM__";
    let script = format!(
        "printf '%s%s%s%s%s%s%s%s%s' \
           '{d}' \"$PATH\" \
           '{d}' \"${{HTTP_PROXY:-$http_proxy}}\" \
           '{d}' \"${{HTTPS_PROXY:-$https_proxy}}\" \
           '{d}' \"${{NO_PROXY:-$no_proxy}}\" \
           '{d}'",
        d = DELIM,
    );

    let mut child = match Command::new(&shell)
        .args(["-l", "-i", "-c", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c)  => c,
        Err(_) => return CapturedEnv::default(),
    };

    // 5s ceiling: a pathological .bashrc (blocks on network, waits for tty)
    // must not freeze the first chat/CLI spawn forever.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    return CapturedEnv::default();
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => return CapturedEnv::default(),
        }
    }

    let output = match child.wait_with_output() {
        Ok(o) if o.status.success() => o,
        _ => return CapturedEnv::default(),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let Some(start) = stdout.find(DELIM) else { return CapturedEnv::default(); };
    let parts: Vec<&str> = stdout[start + DELIM.len()..].split(DELIM).collect();
    let get = |i: usize| {
        parts.get(i)
             .map(|s| s.trim())
             .filter(|s| !s.is_empty())
             .map(|s| s.to_string())
    };

    CapturedEnv {
        path:        get(0),
        http_proxy:  get(1),
        https_proxy: get(2),
        no_proxy:    get(3),
    }
}

// ───────────────────────────────────────────────────────────────────────────
// FreeBSD
// ───────────────────────────────────────────────────────────────────────────

#[cfg(target_os = "freebsd")]
fn capture_impl() -> CapturedEnv {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    // /bin/sh is the base-system shell and the default for new accounts; bash
    // lives in ports and is frequently absent.  Honor $SHELL when it is set.
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());

    // csh is still root's shell in the base system and tcsh remains a common
    // user choice, so this branch cannot assume a POSIX shell the way the
    // macOS and Linux branches do.  Two consequences shape what follows:
    //
    //   1. We ask for `env` instead of a printf script.  `${HTTP_PROXY:-...}`
    //      is a syntax error under csh, which would abort the script and leave
    //      us silently capturing nothing.  `env` is an external command and
    //      behaves identically under sh, csh and tcsh.
    //   2. tcsh rejects `-l` unless it is the only flag, so csh-family shells
    //      get a bare `-c`.  That still picks up PATH: csh sources ~/.cshrc for
    //      non-interactive shells too, and FreeBSD's stock ~/.cshrc sets PATH.
    let is_csh = std::path::Path::new(&shell)
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| n == "csh" || n == "tcsh")
        .unwrap_or(false);
    let args: &[&str] = if is_csh { &["-c", "env"] } else { &["-l", "-i", "-c", "env"] };

    let mut child = match Command::new(&shell)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c)  => c,
        Err(_) => return CapturedEnv::default(),
    };

    // 5s ceiling: a pathological rc file (blocks on network, waits for tty)
    // must not freeze the first chat/CLI spawn forever.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    return CapturedEnv::default();
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => return CapturedEnv::default(),
        }
    }

    let output = match child.wait_with_output() {
        Ok(o) if o.status.success() => o,
        _ => return CapturedEnv::default(),
    };

    // `env` emits KEY=VALUE per line.  Uppercase wins over lowercase when a
    // shell exports both spellings of a proxy var.
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut captured = CapturedEnv::default();
    for line in stdout.lines() {
        let Some((key, value)) = line.split_once('=') else { continue };
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        let slot = match key {
            "PATH"                      => &mut captured.path,
            "HTTP_PROXY"  | "http_proxy"  => &mut captured.http_proxy,
            "HTTPS_PROXY" | "https_proxy" => &mut captured.https_proxy,
            "NO_PROXY"    | "no_proxy"    => &mut captured.no_proxy,
            _ => continue,
        };
        if slot.is_none() || key.starts_with(|c: char| c.is_ascii_uppercase()) {
            *slot = Some(value.to_string());
        }
    }
    captured
}

// ───────────────────────────────────────────────────────────────────────────
// Windows
// ───────────────────────────────────────────────────────────────────────────

#[cfg(windows)]
fn capture_impl() -> CapturedEnv {
    // Windows GUI apps inherit the full user environment from the registry,
    // so there is nothing to capture — PATH already resolves `claude`, and
    // proxy vars set system-wide are already visible.
    CapturedEnv::default()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::EnvGuard;

    const PROXY_VARS: [&str; 6] =
        ["HTTP_PROXY", "http_proxy", "HTTPS_PROXY", "https_proxy", "NO_PROXY", "no_proxy"];

    /// Sets the preference overrides for one test and puts the old ones back after.
    struct Overrides(ProxyOverrides);

    impl Overrides {
        fn set(http: Option<&str>, https: Option<&str>, no_proxy: Option<&str>) -> Self {
            let previous = get_proxy_overrides();
            set_proxy_overrides(http.map(str::to_string), https.map(str::to_string), no_proxy.map(str::to_string));
            Overrides(previous)
        }
    }

    impl Drop for Overrides {
        fn drop(&mut self) {
            let p = self.0.clone();
            set_proxy_overrides(p.http_proxy, p.https_proxy, p.no_proxy);
        }
    }

    /// No proxy variable in the process environment, and no preference override.
    fn clean_slate() -> (EnvGuard, Overrides) {
        let mut env = EnvGuard::lock();
        for key in PROXY_VARS {
            env.remove(key);
        }
        (env, Overrides::set(None, None, None))
    }

    fn captured(path: Option<&str>, http: Option<&str>, https: Option<&str>, no_proxy: Option<&str>) -> CapturedEnv {
        CapturedEnv {
            path: path.map(str::to_string),
            http_proxy: http.map(str::to_string),
            https_proxy: https.map(str::to_string),
            no_proxy: no_proxy.map(str::to_string),
        }
    }

    fn pairs(v: &[(&'static str, &str)]) -> Vec<(&'static str, String)> {
        v.iter().map(|(k, v)| (*k, v.to_string())).collect()
    }

    #[test]
    fn an_empty_no_proxy_gets_all_three_loopback_names() {
        assert_eq!(ensure_localhost_in_no_proxy(None), "localhost,127.0.0.1,::1");
        assert_eq!(ensure_localhost_in_no_proxy(Some(String::new())), "localhost,127.0.0.1,::1");
    }

    #[test]
    fn missing_loopback_names_go_in_front_of_the_users_list() {
        assert_eq!(
            ensure_localhost_in_no_proxy(Some("corp.example.com".into())),
            "localhost,127.0.0.1,::1,corp.example.com"
        );
        assert_eq!(ensure_localhost_in_no_proxy(Some("localhost".into())), "127.0.0.1,::1,localhost");
    }

    #[test]
    fn a_complete_no_proxy_is_left_as_it_is_whatever_its_case() {
        assert_eq!(
            ensure_localhost_in_no_proxy(Some("LOCALHOST,127.0.0.1,::1".into())),
            "LOCALHOST,127.0.0.1,::1"
        );
    }

    #[test]
    fn a_host_that_merely_contains_a_loopback_name_does_not_count() {
        // Each of these contains "localhost", "127.0.0.1" or "::1" as text, and is
        // none of them, so all three loopback names still have to go in front.
        assert_eq!(
            ensure_localhost_in_no_proxy(Some("notlocalhost.corp,127.0.0.10,fe80::1".into())),
            "localhost,127.0.0.1,::1,notlocalhost.corp,127.0.0.10,fe80::1"
        );
    }

    #[test]
    fn loopback_entries_are_recognised_with_spaces_around_them() {
        assert_eq!(
            ensure_localhost_in_no_proxy(Some(" localhost , 127.0.0.1 ,::1".into())),
            " localhost , 127.0.0.1 ,::1"
        );
    }

    #[test]
    fn a_proxy_setting_comes_from_the_preference_then_the_environment_then_the_shell() {
        // Keys of the test's own, so no real proxy variable is read or changed.
        const UPPER: &str = "CLAUDE_ECLIPSE_TEST_PROXY_UPPER";
        const LOWER: &str = "CLAUDE_ECLIPSE_TEST_PROXY_LOWER";
        let keys = [UPPER, LOWER];
        let shell = Some("http://shell".to_string());
        let mut env = EnvGuard::lock();
        env.set(UPPER, "http://env-upper");
        env.set(LOWER, "http://env-lower");

        let pref = Some("http://pref".to_string());
        assert_eq!(resolve_proxy_var(&pref, &keys, &shell).as_deref(), Some("http://pref"));

        let empty_pref = Some(String::new());
        assert_eq!(
            resolve_proxy_var(&empty_pref, &keys, &shell).as_deref(),
            Some("http://env-upper"),
            "an empty preference falls through, and the first key wins"
        );

        env.set(UPPER, "");
        assert_eq!(resolve_proxy_var(&None, &keys, &shell).as_deref(), Some("http://env-lower"));

        env.remove(LOWER);
        assert_eq!(resolve_proxy_var(&None, &keys, &shell).as_deref(), Some("http://shell"));
        assert_eq!(resolve_proxy_var(&None, &keys, &None), None);
    }

    #[test]
    fn only_the_captured_path_is_injected_when_no_proxy_is_set() {
        let _slate = clean_slate();
        let env = captured(Some("/opt/bin:/usr/bin"), None, None, None);
        assert_eq!(env.to_inject(), pairs(&[("PATH", "/opt/bin:/usr/bin")]));
    }

    #[test]
    fn an_active_proxy_brings_loopback_into_no_proxy() {
        let _slate = clean_slate();
        let env = captured(None, None, Some("http://proxy:8080"), None);
        assert_eq!(
            env.to_inject(),
            pairs(&[("HTTPS_PROXY", "http://proxy:8080"), ("NO_PROXY", "localhost,127.0.0.1,::1")])
        );
    }

    #[test]
    fn the_preference_beats_the_shell_and_no_proxy_keeps_the_users_hosts() {
        let (_env, _overrides) = clean_slate();
        let _pref = Overrides::set(Some("http://pref:3128"), None, None);
        let env = captured(None, Some("http://shell:8080"), None, Some("corp.example.com"));
        assert_eq!(
            env.to_inject(),
            pairs(&[
                ("HTTP_PROXY", "http://pref:3128"),
                ("NO_PROXY", "localhost,127.0.0.1,::1,corp.example.com"),
            ])
        );
    }

    #[test]
    fn no_proxy_alone_passes_through_unchanged() {
        let _slate = clean_slate();
        let env = captured(None, None, None, Some("corp.example.com"));
        assert_eq!(env.to_inject(), pairs(&[("NO_PROXY", "corp.example.com")]));
    }
}
