//! Web session history — the History panel's **Web** tab.
//!
//! Lists the conversations this account has on claude.ai via
//! `GET https://api.anthropic.com/v1/code/sessions`, so the panel can show the
//! same Local/Web split the VS Code extension shows.
//!
//! This is a plain REST list. It does NOT touch Remote Control — no bridge
//! socket, no device attestation, no consent prompt — so it stands alone.
//!
//! # Credential containment
//!
//! The OAuth access token is the whole reason this module lives in Rust rather
//! than Java. The rules it holds to, in order of how much they actually buy:
//!
//!   1. **We never store it.** It is read at fetch time, used for exactly one
//!      request, and zeroized ([`Secret`]). There is no copy of ours to protect
//!      at rest, which is worth more than encrypting one would be.
//!   2. **It never crosses JNI.** Java receives display fields only, so the
//!      token can't reach an SWT `String`, the Eclipse error log, or a heap
//!      dump taken from the JVM side.
//!   3. **It is never logged.** The debug lines below print lengths and
//!      outcomes, never bytes; [`Secret`]'s `Debug` redacts.
//!
//! What DOES get encrypted is the cached session list (see [`disk_cache`]) —
//! titles like "Q3 pricing model" leak what the user works on, and unlike the
//! token that cache has to survive a restart to be worth having. The key comes
//! from the OS, never from this binary.

use std::io::Read;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// The CLI's own `BASE_API_URL`. Note this is `api.anthropic.com`, NOT
/// `claude.ai` — the latter is `CLAUDE_AI_ORIGIN` and is only ever used to
/// build links a human clicks.
pub(crate) const BASE_API_URL: &str = "https://api.anthropic.com";

/// Verified from `claude.exe` 2.1.251 (`fetchCodeSessionsFromSessionsAPI`).
/// The list call takes NO query parameters — one page, server default.
const SESSIONS_PATH: &str = "/v1/code/sessions";

/// The CLI sends this on every first-party call; the resolver knows
/// `claude_code_cli`, `_vscode`, `_remote`, `_sdk` and `_mcp`. There is no
/// Eclipse value, so we send the CLI's — we are asking on its behalf, with the
/// credential it minted.
const CLIENT_PLATFORM: &str = "claude_code_cli";

const ANTHROPIC_VERSION: &str = "2023-06-01";

/// How long a fetched list stays fresh. Reopening the panel inside this window
/// renders from memory instead of hitting the network.
const CACHE_TTL_MS: u64 = 60_000;

const CONNECT_TIMEOUT_SECS: u64 = 10;
const CALL_TIMEOUT_SECS: u64 = 20;

// ---------------------------------------------------------------------------
// Secret — a String that is wiped when it drops.
// ---------------------------------------------------------------------------

/// Owns a credential for as long as one request takes, then scrubs it.
///
/// Hand-rolled rather than pulling in `zeroize`: it is a dozen lines, and this
/// crate ships to nine targets where every added dependency is another
/// cross-compile that can break.
///
/// `write_volatile` plus a `SeqCst` fence is the standard shape — the volatile
/// write can't be elided as a dead store, and the fence stops it being sunk
/// past the deallocation.
pub(crate) struct Secret(String);

impl Secret {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
    fn len(&self) -> usize {
        self.0.len()
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        // Safety: zeros are valid UTF-8, so the String stays well-formed for
        // the remainder of its (immediately ending) life.
        let bytes = unsafe { self.0.as_bytes_mut() };
        for b in bytes.iter_mut() {
            unsafe { std::ptr::write_volatile(b, 0) };
        }
        std::sync::atomic::fence(std::sync::atomic::Ordering::SeqCst);
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Secret(<{} bytes redacted>)", self.0.len())
    }
}

// ---------------------------------------------------------------------------
// Home directory
// ---------------------------------------------------------------------------

fn home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("USERPROFILE").map(PathBuf::from)
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME").map(PathBuf::from)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::env::var_os("HOME").map(PathBuf::from)
    }
}

// ---------------------------------------------------------------------------
// Credential reading
// ---------------------------------------------------------------------------

/// What the credential store had to say. `expires_at_ms` is the CLI's own
/// `expiresAt` (epoch ms), so we can tell "stale, go refresh" from "gone".
pub(crate) struct Credential {
    pub(crate) token: Secret,
    expires_at_ms: u64,
}

impl Credential {
    /// True when the CLI's own clock says this access token is past its life.
    /// Checked locally so an expired token costs us a refresh, not a round trip.
    pub(crate) fn is_expired(&self) -> bool {
        self.expires_at_ms != 0 && now_ms() >= self.expires_at_ms
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Reads the OAuth access token from wherever this platform's CLI keeps it.
///
/// Windows, Linux and FreeBSD: `~/.claude/.credentials.json`, plaintext — the
/// CLI writes it that way and we only read it.
///
/// macOS: the login Keychain, which is the store there; the file does not
/// normally exist. Shelled out through `security` because that is the CLI's own
/// read path, and it leaves the OS — not us — deciding whether this process may
/// see the secret.
/// Whether the CLI is signed in with a claude.ai account — an OAuth credential,
/// which is what Claude in Chrome needs and what the extension checks for
/// (`authMethod === "claudeai"`). An API-key login has none. Follows
/// `CLAUDE_CONFIG_DIR` through [`read_credential`].
///
/// Remembered for a minute: the `+` menu asks every time it opens, and on macOS
/// each read runs `security` against the keychain.
pub fn has_claude_ai_login() -> bool {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    static CACHE: Mutex<Option<(Instant, bool)>> = Mutex::new(None);
    const TTL: Duration = Duration::from_secs(60);
    let mut cache = CACHE.lock().unwrap_or_else(|p| p.into_inner());
    if let Some((at, known)) = *cache {
        if at.elapsed() < TTL {
            return known;
        }
    }
    let known = read_credential().is_some();
    *cache = Some((Instant::now(), known));
    known
}

pub(crate) fn read_credential() -> Option<Credential> {
    #[cfg(target_os = "macos")]
    {
        if let Some(c) = read_credential_keychain() {
            return Some(c);
        }
        // A Keychain miss is not proof of being signed out: a user who set
        // CLAUDE_CONFIG_DIR, or who came from an older CLI, still has the file.
        read_credential_file()
    }
    #[cfg(windows)]
    {
        read_credential_file()
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        read_credential_file()
    }
}

fn credentials_path() -> Option<PathBuf> {
    // The CLI honours CLAUDE_CONFIG_DIR for the whole config tree; follow it so
    // a user who relocated their config doesn't look permanently signed out.
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR") {
        return Some(PathBuf::from(dir).join(".credentials.json"));
    }
    Some(home_dir()?.join(".claude").join(".credentials.json"))
}

fn read_credential_file() -> Option<Credential> {
    let path = credentials_path()?;
    let raw = std::fs::read_to_string(&path).ok()?;
    parse_credential(&raw)
}

/// macOS only. `security` prints the secret on stdout, so the child's output is
/// parsed straight into a [`Secret`] and never logged.
///
/// **The account selector is a preference, not a requirement.** The item is
/// stored under service `Claude Code-credentials`, and *sometimes* under account
/// `claude-code-user`. It is not always: on a login created by an older CLI the
/// account is the macOS user's short name instead, and demanding
/// `-a claude-code-user` there makes `security` answer errSecItemNotFound — a
/// perfectly good credential, invisible. That is the whole macOS inbound-message
/// failure: the account-qualified read is tried first (it disambiguates a machine
/// carrying several items from successive CLI versions, where matching on the
/// service alone can hand back a stale token), and when it finds nothing we fall
/// back to the service on its own rather than give up.
///
/// **And it cannot be allowed to hang.** A keychain item whose ACL does not
/// already trust `security` makes this call sit on a GUI prompt, and the caller
/// here is a worker thread that has a message waiting to be drawn. Give up and
/// let the file path answer instead: being slow is worse than being wrong,
/// because the fallback is right.
#[cfg(target_os = "macos")]
fn read_credential_keychain() -> Option<Credential> {
    // The service name has moved across CLI versions; try the known ones.
    const SERVICES: [&str; 2] = ["Claude Code-credentials", "Claude Code"];
    const ACCOUNT: &str = "claude-code-user";
    for service in SERVICES {
        // Preferred selector first, then the service on its own.
        for account in [Some(ACCOUNT), None] {
            match run_security(service, account) {
                Ok(raw) => {
                    if let Some(c) = parse_credential(raw.trim()) {
                        if crate::is_debug() {
                            eprintln!(
                                "[web-history] keychain hit: service {service}, account {}",
                                account.unwrap_or("<any>")
                            );
                        }
                        return Some(c);
                    }
                    if crate::is_debug() {
                        eprintln!(
                            "[web-history] keychain item {service} is not an OAuth credential"
                        );
                    }
                }
                Err(why) => {
                    if crate::is_debug() {
                        eprintln!(
                            "[web-history] keychain {service} (account {}): {why}",
                            account.unwrap_or("<any>")
                        );
                    }
                }
            }
        }
    }
    None
}

/// One `security find-generic-password` run, bounded in time. Returns the raw
/// stdout, or a short reason for the debug log — never the secret itself.
#[cfg(target_os = "macos")]
fn run_security(service: &str, account: Option<&str>) -> Result<String, String> {
    use std::process::{Command, Stdio};
    const TIMEOUT_MS: u64 = 5_000;
    const POLL_MS: u64 = 50;

    // `-a` is omitted entirely when there is no account to match on; passing an
    // empty string would match an item whose account really is "".
    let mut args: Vec<&str> = Vec::with_capacity(6);
    args.push("find-generic-password");
    if let Some(a) = account {
        args.push("-a");
        args.push(a);
    }
    args.extend_from_slice(&["-s", service, "-w"]);

    let mut child = Command::new("security")
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not run security: {e}"))?;

    let mut waited = 0;
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) => {}
            Err(e) => return Err(format!("security failed: {e}")),
        }
        if waited >= TIMEOUT_MS {
            let _ = child.kill();
            let _ = child.wait();
            return Err("security did not answer (waiting on the keychain?)".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(POLL_MS));
        waited += POLL_MS;
    };

    let out = child
        .wait_with_output()
        .map_err(|e| format!("could not read security: {e}"))?;
    if !status.success() {
        // `security` names the OSStatus on stderr (errSecItemNotFound,
        // errSecInteractionNotAllowed, …) — the one thing that says WHICH of the
        // ways this can fail actually happened.
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Pulls `claudeAiOauth.accessToken` / `.expiresAt` out of the credentials JSON.
///
/// Returns `None` for an API-key-only login as well as for no login at all:
/// this endpoint rejects API keys outright ("API key authentication is not
/// sufficient"), so from the Web tab's point of view they are the same state.
fn parse_credential(raw: &str) -> Option<Credential> {
    let v: serde_json::Value = serde_json::from_str(raw).ok()?;
    let oauth = v.get("claudeAiOauth")?;
    let token = oauth.get("accessToken")?.as_str()?;
    if token.is_empty() {
        return None;
    }
    let expires_at_ms = oauth.get("expiresAt").and_then(|e| e.as_u64()).unwrap_or(0);
    Some(Credential {
        token: Secret(token.to_string()),
        expires_at_ms,
    })
}

// ---------------------------------------------------------------------------
// The call
// ---------------------------------------------------------------------------

/// Why a call to the sessions API didn't produce a body.
///
/// `Unauthorized` is kept separate from `Failed` because it is the only outcome
/// worth a token refresh and a second attempt.
pub(crate) enum ApiError {
    /// No OAuth credential at all — including API-key logins, which this API
    /// refuses anyway.
    SignedOut,
    /// Refused even after the CLI rotated the token. A real sign-in problem.
    Expired,
    Failed(String),
}

impl ApiError {
    /// The `state` this maps to in the display JSON Java receives.
    fn state(&self) -> &'static str {
        match self {
            ApiError::SignedOut => "signed-out",
            ApiError::Expired => "expired",
            ApiError::Failed(_) => "error",
        }
    }
}

struct SessionRow {
    id: String,
    title: String,
    status: String,
    repo: String,
    timestamp: String,
}

/// One authenticated GET against the sessions API, returning the raw body.
///
/// **The single place a credential is ever spent.** Everything else in this
/// crate that talks to the sessions API goes through here, so the containment
/// rules in the module header hold for all of it rather than for one call site:
/// the token is read, used, and dropped inside this function.
///
/// Handles the stale-token dance too — a credential the CLI already considers
/// dead skips straight to a refresh, and a 401/403 buys exactly one retry after
/// the CLI has rotated it. Callers see a body or an [`ApiError`], never a
/// half-handled auth state.
///
/// `path` is absolute-from-root, e.g. `/v1/code/sessions`.
pub(crate) fn api_get(claude_cmd: &str, path: &str) -> Result<String, ApiError> {
    let url = format!("{}{}", BASE_API_URL, path);

    let cred = read_credential().ok_or(ApiError::SignedOut)?;
    if crate::is_debug() {
        eprintln!(
            "[web-history] GET {} (token {} bytes, expired={})",
            path,
            cred.token.len(),
            cred.is_expired()
        );
    }

    // A token the CLI itself considers dead isn't worth a round trip.
    if !cred.is_expired() {
        match get_once(&url, &cred.token) {
            Ok(body) => return Ok(body),
            Err(RawError::Unauthorized) => {}
            Err(RawError::Failed(r)) => return Err(ApiError::Failed(r)),
        }
    }
    drop(cred);

    refresh_credential(claude_cmd);
    let cred = read_credential().ok_or(ApiError::Expired)?;
    match get_once(&url, &cred.token) {
        Ok(body) => Ok(body),
        // Refused twice, the second time with a freshly rotated token: this is
        // a real sign-in problem (or an untrusted device), not a stale token.
        Err(RawError::Unauthorized) => {
            if crate::is_debug() {
                eprintln!("[web-history] still unauthorized after refresh");
            }
            Err(ApiError::Expired)
        }
        Err(RawError::Failed(r)) => Err(ApiError::Failed(r)),
    }
}

enum RawError {
    Unauthorized,
    Failed(String),
}

fn get_once(url: &str, token: &Secret) -> Result<String, RawError> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(CONNECT_TIMEOUT_SECS))
        .timeout(std::time::Duration::from_secs(CALL_TIMEOUT_SECS))
        .build();

    let resp = agent
        .get(url)
        .set("Authorization", &format!("Bearer {}", token.as_str()))
        .set("Content-Type", "application/json")
        .set("anthropic-version", ANTHROPIC_VERSION)
        .set("anthropic-client-platform", CLIENT_PLATFORM)
        .call();

    match resp {
        Ok(r) => {
            let mut body = String::new();
            if r.into_reader().read_to_string(&mut body).is_err() {
                return Err(RawError::Failed("couldn't read the response".into()));
            }
            Ok(body)
        }
        Err(ureq::Error::Status(401, _)) | Err(ureq::Error::Status(403, _)) => {
            Err(RawError::Unauthorized)
        }
        Err(ureq::Error::Status(code, _)) => Err(RawError::Failed(format!("HTTP {}", code))),
        Err(ureq::Error::Transport(t)) => Err(RawError::Failed(transport_reason(&t))),
    }
}

/// Gets the CLI to rotate its own token.
///
/// We deliberately do not run the OAuth refresh ourselves. Refresh tokens
/// **rotate** — the server hands back a new one and the CLI writes it to the
/// credentials file. Two writers racing that file is how a user ends up logged
/// out of a CLI they never touched, so the CLI stays the only writer and we
/// read what it leaves.
///
/// The trigger is `claude -p /usage`: a first-party call that costs the user no
/// quota, reusing the probe plumbing `fetch_usage` already has — its own temp
/// working directory, `CREATE_NO_WINDOW`, and the transcript purge that keeps
/// these probes out of `/resume`.
fn refresh_credential(claude_cmd: &str) {
    if crate::is_debug() {
        eprintln!("[web-history] refreshing credential via CLI probe");
    }
    let _ = crate::chat::fetch_usage(claude_cmd, "");
}

/// Keeps the user-facing reason short, and free of the request URL that ureq's
/// own `Display` would splice in.
fn transport_reason(t: &ureq::Transport) -> String {
    match t.kind() {
        ureq::ErrorKind::Dns => "couldn't resolve api.anthropic.com".into(),
        ureq::ErrorKind::ConnectionFailed => "couldn't reach api.anthropic.com".into(),
        _ => "network error".into(),
    }
}

/// Maps the response body to display rows.
///
/// Two things here are easy to get wrong, and are taken from the CLI's own
/// mapping rather than guessed:
///   * the time shown is **`last_event_at`**, not `updated_at`;
///   * the status shown is **`worker_status`**, unless the session is archived.
fn parse_sessions(body: &str) -> Option<Vec<SessionRow>> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let arr = v.get("data")?.as_array()?;
    let mut rows = Vec::with_capacity(arr.len());
    for s in arr {
        let Some(id) = s.get("id").and_then(|x| x.as_str()) else {
            continue;
        };
        let title = s
            .get("title")
            .and_then(|x| x.as_str())
            .filter(|t| !t.is_empty())
            .unwrap_or("Untitled");
        let archived = s.get("status").and_then(|x| x.as_str()) == Some("archived");
        let status = if archived {
            "archived"
        } else {
            s.get("worker_status")
                .and_then(|x| x.as_str())
                .unwrap_or("idle")
        };
        let timestamp = s
            .get("last_event_at")
            .and_then(|x| x.as_str())
            .or_else(|| s.get("created_at").and_then(|x| x.as_str()))
            .unwrap_or("");
        rows.push(SessionRow {
            id: id.to_string(),
            title: title.to_string(),
            status: status.to_string(),
            repo: repo_label(s).unwrap_or_default(),
            timestamp: timestamp.to_string(),
        });
    }
    Some(rows)
}

/// The `config.sources[]` entry of `type: "git_repository"`, rendered
/// `owner/name` the way the extension's repo-mismatch dialog renders it.
fn repo_label(s: &serde_json::Value) -> Option<String> {
    let sources = s.get("config")?.get("sources")?.as_array()?;
    let git = sources
        .iter()
        .find(|x| x.get("type").and_then(|t| t.as_str()) == Some("git_repository"))?;
    let name = git.get("name").and_then(|x| x.as_str()).unwrap_or("");
    let owner = git
        .get("owner")
        .and_then(|o| o.get("login"))
        .and_then(|x| x.as_str())
        .unwrap_or("");
    if name.is_empty() {
        return None;
    }
    Some(if owner.is_empty() {
        name.to_string()
    } else {
        format!("{}/{}", owner, name)
    })
}

fn rows_to_json(rows: &[SessionRow]) -> String {
    let arr: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            serde_json::json!({
                "id": r.id,
                "title": r.title,
                "status": r.status,
                "repo": r.repo,
                "timestamp": r.timestamp,
            })
        })
        .collect();
    serde_json::json!({ "state": "ok", "sessions": arr }).to_string()
}

fn state_only(state: &str) -> String {
    serde_json::json!({ "state": state, "sessions": [] }).to_string()
}

fn error_json(message: &str) -> String {
    serde_json::json!({ "state": "error", "sessions": [], "message": message }).to_string()
}


// ---------------------------------------------------------------------------
// Cache
// ---------------------------------------------------------------------------

struct CacheEntry {
    fetched_ms: u64,
    json: String,
}

static MEM_CACHE: OnceLock<Mutex<Option<CacheEntry>>> = OnceLock::new();

fn mem_cache() -> &'static Mutex<Option<CacheEntry>> {
    MEM_CACHE.get_or_init(|| Mutex::new(None))
}

fn cache_store(json: &str) {
    if let Ok(mut g) = mem_cache().lock() {
        *g = Some(CacheEntry {
            fetched_ms: now_ms(),
            json: json.to_string(),
        });
    }
    disk_cache::store(json);
}

/// Returns the cached list while it is still inside [`CACHE_TTL_MS`].
///
/// On the first call of a process it warms from the encrypted disk cache but
/// deliberately reports *not* fresh, so the panel can paint immediately from
/// the warmed copy while the caller still goes and refetches.
fn cache_fresh() -> Option<String> {
    let mut g = mem_cache().lock().ok()?;
    if let Some(e) = g.as_ref() {
        if now_ms().saturating_sub(e.fetched_ms) < CACHE_TTL_MS {
            return Some(e.json.clone());
        }
        return None;
    }
    if let Some(json) = disk_cache::load() {
        *g = Some(CacheEntry {
            fetched_ms: 0,
            json,
        });
    }
    None
}

/// The last list rendered, still available after a restart.
///
/// Returns whatever the caches hold regardless of age — for painting the panel
/// instantly while a real fetch runs behind it. [`cache_fresh`] is the one that
/// decides whether a fetch can be skipped.
fn cache_any() -> Option<String> {
    let mut g = mem_cache().lock().ok()?;
    if let Some(e) = g.as_ref() {
        return Some(e.json.clone());
    }
    let json = disk_cache::load()?;
    *g = Some(CacheEntry {
        fetched_ms: 0,
        json: json.clone(),
    });
    Some(json)
}

/// The session list at rest.
///
/// Session titles are the user's work — client names, unreleased features,
/// whatever they asked Claude about. On disk they get encrypted with a key the
/// OS holds and ties to the logged-in user, so the file is inert if copied to
/// another account or another machine. **The key never comes from this
/// binary**; a key we shipped would be a key an attacker also has.
///
/// Windows has DPAPI, which is exactly this primitive and needs no dependency.
/// Linux and FreeBSD have no equivalent we can rely on without dragging in
/// libsecret — a runtime `.so` dependency across five of our targets, and
/// absent on a headless box — and macOS keeps credentials in the Keychain
/// rather than on disk at all. Rather than bake a key into the binary and call
/// that encryption, those platforms simply **do not write this cache**: they
/// keep the in-memory one and re-fetch after a restart. One round trip is a
/// cheaper price than a false claim.
mod disk_cache {
    #[cfg(windows)]
    pub(super) fn store(json: &str) {
        let Some(path) = path() else { return };
        let Some(blob) = win_dpapi::protect(json.as_bytes()) else {
            return;
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&path, &blob);
        if crate::is_debug() {
            eprintln!("[web-history] cached {} encrypted bytes", blob.len());
        }
    }

    #[cfg(windows)]
    pub(super) fn load() -> Option<String> {
        let path = path()?;
        let blob = std::fs::read(&path).ok()?;
        // A failure here means the blob was written by another user or another
        // machine — treat it as absent rather than an error worth surfacing.
        let plain = win_dpapi::unprotect(&blob)?;
        let s = String::from_utf8(plain).ok()?;
        if crate::is_debug() {
            eprintln!("[web-history] warmed cache from disk");
        }
        Some(s)
    }

    #[cfg(windows)]
    fn path() -> Option<std::path::PathBuf> {
        Some(
            super::home_dir()?
                .join(".claude")
                .join("eclipse")
                .join("web-sessions.bin"),
        )
    }

    #[cfg(target_os = "macos")]
    pub(super) fn store(_json: &str) {}
    #[cfg(target_os = "macos")]
    pub(super) fn load() -> Option<String> {
        None
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    pub(super) fn store(_json: &str) {}
    #[cfg(all(unix, not(target_os = "macos")))]
    pub(super) fn load() -> Option<String> {
        None
    }

    /// Windows DPAPI, called directly rather than through a crate — it is two
    /// functions, and this crate cross-compiles to nine targets where a new
    /// dependency is a new way for the build to break.
    ///
    /// No entropy argument and no `CRYPTPROTECT_LOCAL_MACHINE`: the key is the
    /// logged-in user's, so another account on this same machine can't decrypt
    /// the file even holding the bytes.
    #[cfg(windows)]
    pub(super) mod win_dpapi {
        #[repr(C)]
        struct DataBlob {
            cb_data: u32,
            pb_data: *mut u8,
        }

        #[link(name = "crypt32")]
        extern "system" {
            fn CryptProtectData(
                data_in: *const DataBlob,
                desc: *const u16,
                optional_entropy: *const DataBlob,
                reserved: *mut std::ffi::c_void,
                prompt: *mut std::ffi::c_void,
                flags: u32,
                data_out: *mut DataBlob,
            ) -> i32;
            fn CryptUnprotectData(
                data_in: *const DataBlob,
                desc_out: *mut *mut u16,
                optional_entropy: *const DataBlob,
                reserved: *mut std::ffi::c_void,
                prompt: *mut std::ffi::c_void,
                flags: u32,
                data_out: *mut DataBlob,
            ) -> i32;
        }

        #[link(name = "kernel32")]
        extern "system" {
            fn LocalFree(mem: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
        }

        /// Copies the buffer DPAPI allocated into a Vec and frees the original.
        /// Shared by both directions so neither leaks the LocalAlloc'd block.
        unsafe fn take(out: DataBlob) -> Vec<u8> {
            let v = std::slice::from_raw_parts(out.pb_data, out.cb_data as usize).to_vec();
            LocalFree(out.pb_data as *mut std::ffi::c_void);
            v
        }

        pub(crate) fn protect(plain: &[u8]) -> Option<Vec<u8>> {
            let input = DataBlob {
                cb_data: plain.len() as u32,
                pb_data: plain.as_ptr() as *mut u8,
            };
            let mut out = DataBlob {
                cb_data: 0,
                pb_data: std::ptr::null_mut(),
            };
            let ok = unsafe {
                CryptProtectData(
                    &input,
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    0,
                    &mut out,
                )
            };
            if ok == 0 || out.pb_data.is_null() {
                return None;
            }
            Some(unsafe { take(out) })
        }

        pub(crate) fn unprotect(blob: &[u8]) -> Option<Vec<u8>> {
            let input = DataBlob {
                cb_data: blob.len() as u32,
                pb_data: blob.as_ptr() as *mut u8,
            };
            let mut out = DataBlob {
                cb_data: 0,
                pb_data: std::ptr::null_mut(),
            };
            let ok = unsafe {
                CryptUnprotectData(
                    &input,
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    0,
                    &mut out,
                )
            };
            if ok == 0 || out.pb_data.is_null() {
                return None;
            }
            Some(unsafe { take(out) })
        }
    }
}

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// The last list we rendered, or `""` when there is none.
///
/// Cheap and non-blocking — no file read beyond the cache, no network — so the
/// Web tab can paint something the instant it's clicked and let [`list`] catch
/// up behind it.
pub fn cached() -> String {
    cache_any().unwrap_or_default()
}

/// Lists this account's claude.ai sessions as display JSON:
/// `{state, sessions:[{id,title,status,repo,timestamp}]}`.
///
/// `state` is one of:
///   * `ok` — `sessions` is populated (possibly empty).
///   * `signed-out` — no OAuth credential. Includes API-key logins, which this
///     endpoint refuses anyway.
///   * `expired` — we had a credential, the server rejected it, and a refresh
///     didn't help. The user has to sign in again.
///   * `error` — network or server problem; `message` says which, briefly.
///
/// **Blocking**: does file I/O, one HTTPS round trip, and may spawn the CLI.
/// Java must call it off the UI thread.
///
/// `claude_cmd` is used only by the refresh path.
pub fn list(claude_cmd: &str, force_refresh: bool) -> String {
    if !force_refresh {
        if let Some(cached) = cache_fresh() {
            if crate::is_debug() {
                eprintln!("[web-history] serving cached list");
            }
            return cached;
        }
    }

    let body = match api_get(claude_cmd, SESSIONS_PATH) {
        Ok(b) => b,
        Err(e) => {
            if crate::is_debug() {
                eprintln!("[web-history] list failed: {}", e.state());
            }
            return match e {
                ApiError::Failed(reason) => error_json(&reason),
                other => state_only(other.state()),
            };
        }
    };

    let Some(rows) = parse_sessions(&body) else {
        return error_json("unexpected response");
    };
    if crate::is_debug() {
        eprintln!("[web-history] fetched {} sessions", rows.len());
    }
    let json = rows_to_json(&rows);
    cache_store(&json);
    json
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_oauth_credential() {
        let raw = r#"{"claudeAiOauth":{"accessToken":"tok-abc","expiresAt":1757000000000,
                       "refreshToken":"r","scopes":[]},"organizationUuid":"o"}"#;
        let c = parse_credential(raw).expect("should parse");
        assert_eq!(c.token.as_str(), "tok-abc");
        assert_eq!(c.expires_at_ms, 1757000000000);
    }

    #[test]
    fn api_key_only_login_reads_as_signed_out() {
        // No claudeAiOauth block — and this endpoint rejects API keys anyway.
        assert!(parse_credential(r#"{"primaryApiKey":"sk-ant-xxx"}"#).is_none());
        assert!(parse_credential(r#"{"claudeAiOauth":{"accessToken":""}}"#).is_none());
        assert!(parse_credential("not json").is_none());
    }

    #[test]
    fn secret_debug_never_prints_the_token() {
        let s = Secret("super-secret-value".into());
        let shown = format!("{:?}", s);
        assert!(!shown.contains("super-secret-value"), "leaked: {}", shown);
        assert!(shown.contains("redacted"));
    }

    #[test]
    fn missing_expiry_is_not_treated_as_expired() {
        // expiresAt absent → 0 → we must NOT loop into a refresh on every call.
        let c = parse_credential(r#"{"claudeAiOauth":{"accessToken":"t"}}"#).unwrap();
        assert_eq!(c.expires_at_ms, 0);
        assert!(!c.is_expired());
    }

    /// The two fields the CLI's own mapping picks that are easy to get wrong:
    /// the time is `last_event_at` (not `updated_at`) and the status is
    /// `worker_status` (not `status`) unless archived.
    #[test]
    fn maps_worker_status_and_last_event_at() {
        let body = r#"{"data":[{
            "id":"session_01A","title":"PR #101 & Build Tool",
            "status":"active","worker_status":"working",
            "created_at":"2026-09-01T00:00:00Z",
            "updated_at":"2026-09-02T00:00:00Z",
            "last_event_at":"2026-09-05T10:00:00Z",
            "config":{"sources":[{"type":"git_repository","name":"repo","owner":{"login":"acme"}}]}
        }]}"#;
        let rows = parse_sessions(body).expect("should parse");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, "working");
        assert_eq!(rows[0].timestamp, "2026-09-05T10:00:00Z");
        assert_eq!(rows[0].repo, "acme/repo");
    }

    #[test]
    fn archived_beats_worker_status() {
        let body = r#"{"data":[{"id":"s","status":"archived","worker_status":"idle"}]}"#;
        let rows = parse_sessions(body).unwrap();
        assert_eq!(rows[0].status, "archived");
    }

    #[test]
    fn tolerates_thin_rows() {
        // Only `id` is required; everything else has a defined fallback, and a
        // row with no id is skipped rather than rendered blank.
        let body = r#"{"data":[{"id":"s1"},{"title":"no id here"},{"id":"s2","title":""}]}"#;
        let rows = parse_sessions(body).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].title, "Untitled");
        assert_eq!(rows[1].title, "Untitled");
        assert_eq!(rows[0].repo, "");
        assert_eq!(rows[0].timestamp, "");
    }

    #[test]
    fn falls_back_to_created_at_when_no_events_yet() {
        let body = r#"{"data":[{"id":"s","created_at":"2026-09-01T00:00:00Z"}]}"#;
        let rows = parse_sessions(body).unwrap();
        assert_eq!(rows[0].timestamp, "2026-09-01T00:00:00Z");
    }

    #[test]
    fn repo_without_owner_renders_bare_name() {
        let body = r#"{"data":[{"id":"s","config":{"sources":[
            {"type":"other","name":"x"},
            {"type":"git_repository","name":"solo"}]}}]}"#;
        let rows = parse_sessions(body).unwrap();
        assert_eq!(rows[0].repo, "solo");
    }

    #[test]
    fn rejects_non_list_bodies() {
        assert!(parse_sessions(r#"{"error":{"message":"nope"}}"#).is_none());
        assert!(parse_sessions("").is_none());
    }

    #[test]
    fn display_json_carries_no_credential_fields() {
        let rows = parse_sessions(r#"{"data":[{"id":"s","title":"t"}]}"#).unwrap();
        let json = rows_to_json(&rows).to_lowercase();
        for banned in ["accesstoken", "refreshtoken", "bearer", "authorization"] {
            assert!(
                !json.contains(banned),
                "display JSON must not carry {}: {}",
                banned,
                json
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn dpapi_round_trips() {
        let plain = br#"{"state":"ok","sessions":[]}"#;
        let blob = disk_cache::win_dpapi::protect(plain).expect("protect");
        assert_ne!(&blob[..], &plain[..], "must not store plaintext");
        let back = disk_cache::win_dpapi::unprotect(&blob).expect("unprotect");
        assert_eq!(back, plain);
    }
}
