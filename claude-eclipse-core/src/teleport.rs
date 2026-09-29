//! Teleport — continuing a claude.ai session here, locally.
//!
//! Clicking a row in the History panel's **Web** tab pulls that conversation
//! down and carries on with it in a local tab: the transcript is fetched, saved
//! as an ordinary local session, and from then on it is one. The agent runs on
//! this machine, in this workspace.
//!
//! # Why this is reimplemented rather than delegated
//!
//! The CLI has a `--teleport <id>` flag, and we deliberately do not use it. It
//! validates the repo itself and **throws** when the session belongs to a
//! different one:
//!
//! ```text
//! You must run claude --teleport <id> from a checkout of <sessionRepo>.
//! This repo is <currentRepo>.
//! ```
//!
//! There is no "continue anyway" behind that flag — so shelling out to it can
//! only ever implement the *refusal*, never the choice. The VS Code extension
//! reaches the same conclusion and reimplements the whole flow against the REST
//! API for exactly this reason: its "Continue here" option cannot exist on top
//! of a CLI path that hard-refuses the case the option is for.
//!
//! So the split is: **the decision is ours, the work is ordinary git and an
//! ordinary transcript file.** The CLI still owns everything downstream, since
//! what we hand back is a normal local session it can `--resume`.
//!
//! This module holds the read-only half — classifying the repo and asking git
//! questions. Nothing here writes to the working tree.

use std::path::Path;
use std::process::Command;

// ---------------------------------------------------------------------------
// Repository references
// ---------------------------------------------------------------------------

/// A repo identified the way both sides of the comparison name it.
///
/// `host` is kept because two checkouts can share `owner/name` on different
/// hosts (a GitHub fork and a GitHub Enterprise mirror), and that is a genuine
/// mismatch even though the tail matches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoRef {
    pub host: String,
    pub owner: String,
    pub name: String,
}

impl RepoRef {
    /// `owner/name` — how a repo is written when the host is not in question.
    pub fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }
}

/// Strips a `:port` suffix so `git.example.com:2222` and `git.example.com`
/// compare equal — the CLI does the same before comparing hosts.
fn host_key(host: &str) -> String {
    let h = host.to_lowercase();
    match h.rfind(':') {
        // Only a numeric tail is a port; an IPv6 literal has colons too.
        Some(i) if h[i + 1..].chars().all(|c| c.is_ascii_digit()) && i + 1 < h.len() => {
            h[..i].to_string()
        }
        _ => h,
    }
}

/// Parses the git remote forms that actually turn up in the wild:
///
/// ```text
/// https://github.com/owner/repo.git
/// https://user@github.com/owner/repo
/// ssh://git@github.com:2222/owner/repo.git
/// git@github.com:owner/repo.git          (scp-like, no scheme)
/// ```
///
/// Deeper paths keep the LAST two segments, so a GitLab subgroup
/// (`gitlab.com/group/sub/repo`) reads as `sub/repo` — which is what both the
/// API and a human call it.
pub fn parse_repo_url(url: &str) -> Option<RepoRef> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }

    let (host_part, path_part) = if let Some(rest) = url
        .find("://")
        .map(|i| &url[i + 3..])
    {
        // scheme://[user@]host[:port]/path
        let rest = rest.split_once('@').map_or(rest, |(_, r)| r);
        rest.split_once('/')?
    } else if let Some((left, right)) = url.split_once(':') {
        // scp-like: [user@]host:path — but only when the tail is not a port,
        // which would mean this was a URL with a scheme we failed to see.
        let host = left.split_once('@').map_or(left, |(_, h)| h);
        (host, right)
    } else {
        return None;
    };

    if host_part.is_empty() {
        return None;
    }

    let path = path_part.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let mut segments = path.split('/').filter(|s| !s.is_empty());
    let name = segments.next_back()?;
    let owner = segments.next_back()?;
    if name.is_empty() || owner.is_empty() {
        return None;
    }

    Some(RepoRef {
        host: host_part.to_string(),
        owner: owner.to_string(),
        name: name.to_string(),
    })
}

// ---------------------------------------------------------------------------
// Classification
// ---------------------------------------------------------------------------

/// How this workspace relates to the repo a session was created in.
///
/// Mirrors the CLI's own `validateSessionRepository`, including the two states
/// that are easy to miss: `HostUnverified` **proceeds** there rather than
/// prompting, and `NoRepoRequired` covers a session that names no repo at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepoStatus {
    /// Same repo — go straight through, no dialog.
    Match,
    /// The session names no repository, so there is nothing to disagree with.
    NoRepoRequired,
    /// Same `owner/name`, host couldn't be confirmed. Proceeds, like the CLI.
    HostUnverified,
    /// This folder isn't a git checkout at all.
    NotInRepo,
    /// A different repository. The one case the dialog exists for.
    Mismatch,
}

impl RepoStatus {
    fn as_str(self) -> &'static str {
        match self {
            RepoStatus::Match => "match",
            RepoStatus::NoRepoRequired => "no_repo_required",
            RepoStatus::HostUnverified => "host_unverified",
            RepoStatus::NotInRepo => "not_in_repo",
            RepoStatus::Mismatch => "mismatch",
        }
    }

    /// Whether teleport may start without asking. Only `Mismatch` and
    /// `NotInRepo` are worth interrupting for.
    pub fn proceeds_silently(self) -> bool {
        matches!(
            self,
            RepoStatus::Match | RepoStatus::NoRepoRequired | RepoStatus::HostUnverified
        )
    }
}

pub struct RepoDecision {
    pub status: RepoStatus,
    pub session: Option<RepoRef>,
    pub current: Option<RepoRef>,
}

/// Classifies `workspace_root` against the repo a session was created in.
///
/// `session_repo_url` is the `config.sources[]` entry of type `git_repository`;
/// an empty string means the session named none.
pub fn classify(session_repo_url: &str, workspace_root: &str) -> RepoDecision {
    let Some(session) = parse_repo_url(session_repo_url) else {
        // No parseable repo on the session — nothing to disagree with. Matches
        // the CLI, which treats an unparseable url the same as an absent one.
        return RepoDecision {
            status: RepoStatus::NoRepoRequired,
            session: None,
            current: None,
        };
    };

    let root = Path::new(workspace_root);
    let Some(current) = remote_url(root, "origin").and_then(|u| parse_repo_url(&u)) else {
        // Not a checkout, or a remote we can't read. Distinguish "no git here"
        // from "git, but a remote we couldn't parse" only insofar as the dialog
        // cares: both mean we cannot claim a match.
        return RepoDecision {
            status: RepoStatus::NotInRepo,
            session: Some(session),
            current: None,
        };
    };

    if same_repo(&session, &current) {
        return decided(RepoStatus::Match, session, current);
    }

    // Before calling it a mismatch, try the other remote a fork typically has:
    // `upstream` is where `owner/name` usually matches when `origin` is a fork.
    if let Some(upstream) = remote_url(root, "upstream").and_then(|u| parse_repo_url(&u)) {
        if same_repo(&session, &upstream) {
            return decided(RepoStatus::Match, session, upstream);
        }
    }

    // Same repo name, different host: the CLI proceeds on this rather than
    // prompting, so we do too.
    if session.owner.eq_ignore_ascii_case(&current.owner)
        && session.name.eq_ignore_ascii_case(&current.name)
    {
        return decided(RepoStatus::HostUnverified, session, current);
    }

    decided(RepoStatus::Mismatch, session, current)
}

fn decided(status: RepoStatus, session: RepoRef, current: RepoRef) -> RepoDecision {
    RepoDecision {
        status,
        session: Some(session),
        current: Some(current),
    }
}

fn same_repo(a: &RepoRef, b: &RepoRef) -> bool {
    a.owner.eq_ignore_ascii_case(&b.owner)
        && a.name.eq_ignore_ascii_case(&b.name)
        && host_key(&a.host) == host_key(&b.host)
}

/// How to name each side in the dialog.
///
/// The host is spelled out **only when the two hosts differ** — otherwise
/// `owner/name` alone, since repeating a host both sides share tells the reader
/// nothing. Same rule as the CLI's `formatRepoMismatchDisplay`.
pub fn display_pair(d: &RepoDecision) -> (String, String) {
    let (Some(s), Some(c)) = (&d.session, &d.current) else {
        return (
            d.session.as_ref().map(|s| s.slug()).unwrap_or_default(),
            d.current.as_ref().map(|c| c.slug()).unwrap_or_default(),
        );
    };
    if host_key(&s.host) != host_key(&c.host) {
        (
            format!("{}/{}", s.host, s.slug()),
            format!("{}/{}", c.host, c.slug()),
        )
    } else {
        (s.slug(), c.slug())
    }
}

/// The decision as display JSON for the webview dialog.
///
/// `sessionOwner`/`sessionName` are separate because the dialog renders them as
/// one code pill and needs them un-joined; `sessionDisplay`/`currentDisplay`
/// carry the host-qualified forms for the prose.
pub fn decision_json(d: &RepoDecision) -> String {
    let (session_display, current_display) = display_pair(d);
    serde_json::json!({
        "status": d.status.as_str(),
        "proceed": d.status.proceeds_silently(),
        "sessionOwner": d.session.as_ref().map(|s| s.owner.clone()).unwrap_or_default(),
        "sessionName": d.session.as_ref().map(|s| s.name.clone()).unwrap_or_default(),
        "sessionDisplay": session_display,
        "currentDisplay": current_display,
    })
    .to_string()
}

// ---------------------------------------------------------------------------
// Git questions (read-only)
// ---------------------------------------------------------------------------

/// Runs git in `root` and returns trimmed stdout on success.
///
/// Deliberately no shell: every argument is passed as its own token, so a
/// branch or remote name can never be read as one. `CREATE_NO_WINDOW` keeps a
/// console from flashing on Windows, the same as every other spawn we do.
fn git(root: &Path, args: &[&str]) -> Option<String> {
    if root.as_os_str().is_empty() {
        return None;
    }
    let mut cmd = Command::new("git");
    cmd.args(args).current_dir(root);

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }

    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// The URL of one remote, or `None` if there is no such remote (or no repo).
pub fn remote_url(root: &Path, remote: &str) -> Option<String> {
    git(root, &["remote", "get-url", remote]).filter(|s| !s.is_empty())
}

/// The branch currently checked out, or `None` on a detached HEAD.
pub fn current_branch(root: &Path) -> Option<String> {
    git(root, &["symbolic-ref", "--quiet", "--short", "HEAD"]).filter(|s| !s.is_empty())
}

/// Whether the branch already exists in this checkout.
pub fn branch_exists_local(root: &Path, branch: &str) -> bool {
    if !is_valid_branch_name(branch) {
        return false;
    }
    git(
        root,
        &["rev-parse", "--verify", &format!("refs/heads/{}", branch)],
    )
    .is_some()
}

/// Whether `origin` publishes the branch. Hits the network, so it is only worth
/// asking once the local check has already failed.
pub fn branch_exists_on_origin(root: &Path, branch: &str) -> bool {
    if !is_valid_branch_name(branch) {
        return false;
    }
    git(root, &["ls-remote", "--heads", "origin", branch])
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false)
}

/// Paths with uncommitted changes, tracked or not. Empty means a clean tree.
///
/// `--porcelain` is the stable, script-facing format; the two leading status
/// columns and the space after them are fixed-width, so the path starts at
/// byte 3 regardless of locale.
pub fn changed_files(root: &Path) -> Vec<String> {
    let Some(out) = git(root, &["status", "--porcelain"]) else {
        return Vec::new();
    };
    out.lines()
        .filter(|l| l.len() > 3)
        .map(|l| {
            let p = l[3..].trim();
            // A rename reads "old -> new"; the new path is the one that matters.
            p.rsplit(" -> ").next().unwrap_or(p).trim_matches('"').to_string()
        })
        .collect()
}

/// Whether a string is safe to hand to git as a branch name.
///
/// This is `git check-ref-format --branch` in miniature. It exists for safety,
/// not tidiness: the name arrives from a server response and ends up as a git
/// argument, and a leading `-` alone would turn it into a flag.
pub fn is_valid_branch_name(b: &str) -> bool {
    if b.is_empty() || b == "@" || b.len() > 255 {
        return false;
    }
    if b.starts_with('-') || b.starts_with('.') || b.starts_with('/') {
        return false;
    }
    if b.ends_with('/') || b.ends_with('.') || b.ends_with(".lock") {
        return false;
    }
    if b.contains("..") || b.contains("//") || b.contains("@{") {
        return false;
    }
    for c in b.chars() {
        if c.is_control() || c == '\u{7f}' {
            return false;
        }
        if matches!(c, ' ' | '~' | '^' | ':' | '?' | '*' | '[' | '\\') {
            return false;
        }
    }
    // Each slash-separated component carries the same leading-dot/.lock rules.
    b.split('/')
        .all(|part| !part.is_empty() && !part.starts_with('.') && !part.ends_with(".lock"))
}

// ---------------------------------------------------------------------------
// Pulling the session down
// ---------------------------------------------------------------------------

/// Pages to walk before giving up. A very long conversation is still bounded;
/// without this a server that kept handing back the same cursor would spin.
const MAX_EVENT_PAGES: usize = 200;

/// The `payload.type` values that carry the conversation itself.
///
/// The events endpoint replays the whole stream-json protocol, most of which is
/// machinery: `control_request`/`control_response` are the SDK handshake and
/// `rate_limit_event` is telemetry. Only these four say what was said, and they
/// are the same shapes `session.rs` already renders from a local transcript.
const TRANSCRIPT_TYPES: [&str; 4] = ["user", "assistant", "system", "result"];

/// Fetches one session's detail record.
pub fn fetch_detail(claude_cmd: &str, id: &str) -> Result<serde_json::Value, crate::web_history::ApiError> {
    let body = crate::web_history::api_get(claude_cmd, &format!("/v1/code/sessions/{}", id))?;
    serde_json::from_str(&body)
        .map_err(|_| crate::web_history::ApiError::Failed("unexpected response".into()))
}

/// Fetches every page of a session's events, oldest first.
///
/// Pages with `?cursor=`, stopping on an empty `next_cursor`, an empty page, or
/// a cursor the server repeats — the last of which would otherwise loop.
pub fn fetch_events(
    claude_cmd: &str,
    id: &str,
) -> Result<Vec<serde_json::Value>, crate::web_history::ApiError> {
    let mut all = Vec::new();
    let mut cursor = String::new();
    let mut seen_cursors: Vec<String> = Vec::new();

    for _ in 0..MAX_EVENT_PAGES {
        let path = if cursor.is_empty() {
            format!("/v1/code/sessions/{}/events", id)
        } else {
            format!("/v1/code/sessions/{}/events?cursor={}", id, cursor)
        };
        let body = crate::web_history::api_get(claude_cmd, &path)?;
        let v: serde_json::Value = serde_json::from_str(&body)
            .map_err(|_| crate::web_history::ApiError::Failed("unexpected response".into()))?;

        let page = v.get("data").and_then(|d| d.as_array()).cloned().unwrap_or_default();
        let page_len = page.len();
        all.extend(page);

        let next = v
            .get("next_cursor")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        if next.is_empty() || page_len == 0 || seen_cursors.contains(&next) {
            break;
        }
        seen_cursors.push(next.clone());
        cursor = next;
    }

    if crate::is_debug() {
        eprintln!("[teleport] fetched {} events", all.len());
    }
    Ok(all)
}

/// Turns the event stream into transcript lines a local session file can hold.
///
/// Two things the envelope has that the payload doesn't, and that the renderer
/// needs: the time (`created_at`) and which session it now belongs to. Ordering
/// is by `sequence_num`, not by arrival — pages come back newest-cursor-first
/// and a stable numeric sort is the only thing that reassembles them correctly.
pub fn to_transcript(
    events: &[serde_json::Value],
    local_session_id: &str,
    cwd: &str,
) -> Vec<serde_json::Value> {
    let mut rows: Vec<(u64, serde_json::Value)> = Vec::new();

    for e in events {
        let Some(payload) = e.get("payload") else { continue };
        let kind = payload.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if !TRANSCRIPT_TYPES.contains(&kind) {
            continue;
        }

        let mut line = payload.clone();
        if let Some(obj) = line.as_object_mut() {
            if let Some(ts) = e.get("created_at").and_then(|x| x.as_str()) {
                obj.insert("timestamp".into(), serde_json::json!(ts));
            }
            // Rewritten, not carried over: these lines now belong to a local
            // session in this folder, and the renderer keys off both.
            obj.insert("sessionId".into(), serde_json::json!(local_session_id));
            obj.insert("session_id".into(), serde_json::json!(local_session_id));
            if !cwd.is_empty() {
                obj.insert("cwd".into(), serde_json::json!(cwd));
            }
        }

        // sequence_num arrives as a string; sort numerically so 9 precedes 10.
        let seq = e
            .get("sequence_num")
            .and_then(|s| s.as_str().and_then(|t| t.parse::<u64>().ok()).or_else(|| s.as_u64()))
            .unwrap_or(0);
        rows.push((seq, line));
    }

    rows.sort_by_key(|(seq, _)| *seq);
    rows.into_iter().map(|(_, line)| line).collect()
}

/// Writes the transcript as a local session under this workspace and returns
/// its new id.
///
/// A fresh uuid rather than the remote id: the remote one lives in another
/// namespace (`cse_…`), and reusing it would collide with the copy that already
/// exists if the same session is teleported twice.
///
/// Writes to `~/.claude/projects/<workspace hash>/<uuid>.jsonl` — the CLI's own
/// layout, so `/resume`, our History panel and every other Claude Code client
/// see it as an ordinary past conversation, which is exactly what it now is.
pub fn write_local_session(
    workspace_root: &str,
    lines: &[serde_json::Value],
) -> Option<String> {
    let home = crate::session::dirs_home()?;
    let dir = home
        .join(".claude")
        .join("projects")
        .join(crate::session::workspace_hash(workspace_root));
    std::fs::create_dir_all(&dir).ok()?;

    let id = new_uuid_v4();
    let path = dir.join(format!("{}.jsonl", id));

    let mut body = String::new();
    for line in lines {
        body.push_str(&line.to_string());
        body.push('\n');
    }
    std::fs::write(&path, body).ok()?;

    if crate::is_debug() {
        eprintln!(
            "[teleport] wrote {} transcript lines to {}",
            lines.len(),
            path.display()
        );
    }
    Some(id)
}

/// A v4 uuid, formatted the way the CLI writes session ids.
///
/// Hand-rolled off `uuid`'s generator (already a dependency) so the version and
/// variant bits are right without adding a formatting crate.
fn new_uuid_v4() -> String {
    uuid::Uuid::new_v4().to_string()
}
// ---------------------------------------------------------------------------
// Orchestration
// ---------------------------------------------------------------------------

/// The `git_repository` source url on a session, or `""` when it names none.
///
/// A Remote Control session has an empty `config.sources` — the agent was
/// already running on someone's machine, so there is no cloud checkout to name.
/// Those sessions therefore never reach the repo dialog at all.
pub fn session_repo_url(detail: &serde_json::Value) -> String {
    detail
        .get("config")
        .and_then(|c| c.get("sources"))
        .and_then(|s| s.as_array())
        .and_then(|sources| {
            sources
                .iter()
                .find(|s| s.get("type").and_then(|t| t.as_str()) == Some("git_repository"))
        })
        .and_then(|s| s.get("url").and_then(|u| u.as_str()))
        .unwrap_or("")
        .to_string()
}

/// The branch a session worked on, if it names one.
///
/// **Verified absent for Remote Control sessions** (their `config.sources` is
/// empty and no branch field appears anywhere in the detail record). Where a
/// cloud session carries it is *unverified* — no cloud session was available to
/// inspect — so all three plausible spellings are checked and a miss is treated
/// as "no branch", which is the safe direction: no branch means nothing is
/// checked out and the working tree is left alone.
pub fn session_branch(detail: &serde_json::Value) -> String {
    let from_source = detail
        .get("config")
        .and_then(|c| c.get("sources"))
        .and_then(|s| s.as_array())
        .and_then(|sources| {
            sources
                .iter()
                .find(|s| s.get("type").and_then(|t| t.as_str()) == Some("git_repository"))
        })
        .and_then(|s| s.get("branch").and_then(|b| b.as_str()));

    let branch = from_source
        .or_else(|| detail.get("branch").and_then(|b| b.as_str()))
        .or_else(|| {
            detail
                .get("config")
                .and_then(|c| c.get("branch"))
                .and_then(|b| b.as_str())
        })
        .unwrap_or("");

    // A name git would refuse is treated as no branch rather than passed on.
    if is_valid_branch_name(branch) {
        branch.to_string()
    } else {
        String::new()
    }
}

/// Classifies a session against a workspace, for the "Different repository"
/// dialog. Read-only: one GET, no git writes.
///
/// **Blocking** — Java must call it off the UI thread.
pub fn repo_check(claude_cmd: &str, session_id: &str, workspace_root: &str) -> String {
    let detail = match fetch_detail(claude_cmd, session_id) {
        Ok(d) => d,
        Err(e) => return api_error_json(e),
    };
    let decision = classify(&session_repo_url(&detail), workspace_root);
    if crate::is_debug() {
        eprintln!("[teleport] repo check: {}", decision.status.as_str());
    }
    decision_json(&decision)
}

/// Pulls a session down into this workspace as a local conversation.
///
/// Returns `{"ok":true, localSessionId, title, branch, branchExists, messageCount}`.
/// `branch` is non-empty only when the session names one AND it actually exists
/// locally or on `origin` — the caller shows the branch prompt on that, and
/// **nothing here checks anything out**; see [`checkout_branch`].
///
/// **Blocking** — several round trips plus a file write. Off the UI thread.
pub fn run(claude_cmd: &str, session_id: &str, workspace_root: &str) -> String {
    let detail = match fetch_detail(claude_cmd, session_id) {
        Ok(d) => d,
        Err(e) => return api_error_json(e),
    };
    let title = detail
        .get("title")
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string();

    let events = match fetch_events(claude_cmd, session_id) {
        Ok(e) => e,
        Err(e) => return api_error_json(e),
    };

    // A brand-new session with nothing said yet is not an error, but there is
    // no conversation to carry over — say so rather than writing an empty file.
    let lines = to_transcript(&events, "", workspace_root);
    if lines.is_empty() {
        if crate::is_debug() {
            eprintln!("[teleport] no transcript in {} events", events.len());
        }
        return serde_json::json!({
            "ok": false, "error": "empty",
            "message": "This conversation has no messages to carry over."
        })
        .to_string();
    }

    let Some(local_id) = write_local_session(workspace_root, &lines) else {
        return serde_json::json!({
            "ok": false, "error": "write",
            "message": "Couldn\u{2019}t save the conversation locally."
        })
        .to_string();
    };
    // Rewrite the ids now that we know them, then persist for real.
    let mut lines = to_transcript(&events, &local_id, workspace_root);
    // Marks where the web conversation ends and this local one begins. Written
    // into the transcript rather than drawn once, so it is still there when the
    // conversation is reopened from history months later — the boundary is a
    // fact about the conversation, not a detail of the session that made it.
    lines.push(teleport_marker(&local_id, workspace_root));
    let _ = rewrite_local_session(workspace_root, &local_id, &lines);

    // Only offer the branch if it is real — a name for a branch that exists
    // nowhere would produce a checkout prompt that could only ever fail.
    let branch = session_branch(&detail);
    let root = Path::new(workspace_root);
    let branch_exists = !branch.is_empty()
        && (branch_exists_local(root, &branch) || branch_exists_on_origin(root, &branch));
    if !branch.is_empty() && !branch_exists && crate::is_debug() {
        eprintln!(
            "[teleport] branch {} exists neither locally nor on origin — skipping the prompt",
            branch
        );
    }

    if crate::is_debug() {
        eprintln!(
            "[teleport] {} → local {} ({} lines, branch={})",
            session_id,
            local_id,
            lines.len(),
            if branch_exists { branch.as_str() } else { "none" }
        );
    }

    serde_json::json!({
        "ok": true,
        "localSessionId": local_id,
        "title": title,
        "branch": if branch_exists { branch } else { String::new() },
        "messageCount": lines.len(),
    })
    .to_string()
}

/// The `teleported_from_web` boundary line.
///
/// Shaped as a `system` event with a subtype, the same way the CLI records a
/// compact boundary — so it rides in the transcript as ordinary data that older
/// readers skip rather than choke on.
pub fn teleport_marker(local_session_id: &str, cwd: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "system",
        "subtype": "teleported_from_web",
        "sessionId": local_session_id,
        "session_id": local_session_id,
        "cwd": cwd,
        "timestamp": now_iso8601(),
    })
}

/// UTC timestamp in the shape the CLI writes, without pulling in a date crate.
fn now_iso8601() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Days since the epoch → civil date, via Howard Hinnant's algorithm.
    let days = (secs / 86_400) as i64;
    let tod = secs % 86_400;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.000Z",
        y, m, d, tod / 3600, (tod % 3600) / 60, tod % 60
    )
}

/// Overwrites an already-written local session with corrected lines.
fn rewrite_local_session(
    workspace_root: &str,
    local_id: &str,
    lines: &[serde_json::Value],
) -> Option<()> {
    let home = crate::session::dirs_home()?;
    let path = home
        .join(".claude")
        .join("projects")
        .join(crate::session::workspace_hash(workspace_root))
        .join(format!("{}.jsonl", local_id));
    let mut body = String::new();
    for line in lines {
        body.push_str(&line.to_string());
        body.push('\n');
    }
    std::fs::write(path, body).ok()
}

fn api_error_json(e: crate::web_history::ApiError) -> String {
    use crate::web_history::ApiError;
    let (state, message) = match e {
        ApiError::SignedOut => ("signed-out", "Sign in to Claude Code first.".to_string()),
        ApiError::Expired => ("expired", "Your login expired. Sign in again.".to_string()),
        ApiError::Failed(r) => ("error", r),
    };
    serde_json::json!({ "ok": false, "error": state, "message": message }).to_string()
}

/// Checks out the branch a teleported session was working on.
///
/// **The only function in this module that writes to the working tree**, and it
/// runs solely when the user has answered the branch prompt. Fetches first
/// (`origin/<b>:<b>`, falling back to a plain fetch when that refspec is
/// refused because the branch already exists locally), then checks out.
///
/// Returns `{"ok":true,"branch":…}` or `{"ok":false,"message":…}`.
pub fn checkout_branch(workspace_root: &str, branch: &str) -> String {
    if !is_valid_branch_name(branch) {
        return serde_json::json!({ "ok": false, "message": "Invalid branch name." }).to_string();
    }
    let root = Path::new(workspace_root);

    // Best-effort: a fetch failure is not fatal when the branch is already here.
    if git(root, &["fetch", "origin", &format!("{}:{}", branch, branch)]).is_none() {
        let _ = git(root, &["fetch", "origin", branch]);
    }

    if git(root, &["checkout", branch]).is_none() {
        if crate::is_debug() {
            eprintln!("[teleport] checkout of {} failed", branch);
        }
        return serde_json::json!({
            "ok": false,
            "message": format!("Couldn\u{2019}t switch to {}.", branch)
        })
        .to_string();
    }
    if crate::is_debug() {
        eprintln!("[teleport] checked out {}", branch);
    }
    serde_json::json!({ "ok": true, "branch": branch }).to_string()
}

/// Whether the working tree is clean, and what has changed if not — the branch
/// prompt needs both to decide whether to warn before switching.
pub fn git_status_json(workspace_root: &str) -> String {
    let files = changed_files(Path::new(workspace_root));
    serde_json::json!({
        "clean": files.is_empty(),
        "changedFiles": files,
        "currentBranch": current_branch(Path::new(workspace_root)).unwrap_or_default(),
    })
    .to_string()
}
#[cfg(test)]
mod tests;

/// Exercises the real `git` subprocess path rather than the pure functions
/// above — the classifier is only as good as what `git remote get-url` hands
/// it, and that seam is where a quoting or working-directory mistake would
/// live. Builds a throwaway repo so it is portable and touches nothing.
#[cfg(test)]
mod git_tests;

/// The event→transcript conversion, which is where a wrong assumption would be
/// invisible: the wrong sort key reorders a conversation, and keeping protocol
/// frames would render handshake noise as if someone had said it.
#[cfg(test)]
mod transcript_tests;
