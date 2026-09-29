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
mod tests {
    use super::*;

    fn r(host: &str, owner: &str, name: &str) -> RepoRef {
        RepoRef {
            host: host.into(),
            owner: owner.into(),
            name: name.into(),
        }
    }

    #[test]
    fn parses_the_remote_url_forms_that_occur() {
        assert_eq!(
            parse_repo_url("https://github.com/acme/widgets.git"),
            Some(r("github.com", "acme", "widgets"))
        );
        assert_eq!(
            parse_repo_url("https://github.com/acme/widgets"),
            Some(r("github.com", "acme", "widgets"))
        );
        // scp-like, the form ssh remotes usually take
        assert_eq!(
            parse_repo_url("git@github.com:acme/widgets.git"),
            Some(r("github.com", "acme", "widgets"))
        );
        assert_eq!(
            parse_repo_url("ssh://git@github.com:2222/acme/widgets.git"),
            Some(r("github.com:2222", "acme", "widgets"))
        );
        // a userinfo segment must not be mistaken for the host
        assert_eq!(
            parse_repo_url("https://someone@git.example.com/acme/widgets"),
            Some(r("git.example.com", "acme", "widgets"))
        );
    }

    #[test]
    fn a_subgroup_path_keeps_the_last_two_segments() {
        assert_eq!(
            parse_repo_url("https://gitlab.com/group/sub/widgets.git"),
            Some(r("gitlab.com", "sub", "widgets"))
        );
    }

    #[test]
    fn rejects_what_it_cannot_name() {
        assert!(parse_repo_url("").is_none());
        assert!(parse_repo_url("not a url").is_none());
        assert!(parse_repo_url("https://github.com/onlyone").is_none());
    }

    #[test]
    fn host_comparison_ignores_port_and_case() {
        assert_eq!(host_key("GitHub.com"), "github.com");
        assert_eq!(host_key("git.example.com:2222"), "git.example.com");
        // not a port — must survive intact
        assert_eq!(host_key("git.example.com:branch"), "git.example.com:branch");
    }

    #[test]
    fn no_repo_on_the_session_needs_no_agreement() {
        let d = classify("", "");
        assert_eq!(d.status, RepoStatus::NoRepoRequired);
        assert!(d.status.proceeds_silently());
    }

    #[test]
    fn an_unparseable_session_url_is_treated_as_no_repo() {
        // The CLI does the same rather than blocking on a url it can't read.
        assert_eq!(classify("garbage", "").status, RepoStatus::NoRepoRequired);
    }

    #[test]
    fn same_owner_and_name_on_another_host_is_host_unverified_not_mismatch() {
        let session = r("github.com", "acme", "widgets");
        let current = r("ghe.corp.example", "acme", "widgets");
        assert!(!same_repo(&session, &current));
        // The classifier reaches HostUnverified for this shape, which proceeds.
        assert!(RepoStatus::HostUnverified.proceeds_silently());
    }

    #[test]
    fn only_mismatch_and_not_in_repo_interrupt() {
        assert!(RepoStatus::Match.proceeds_silently());
        assert!(RepoStatus::NoRepoRequired.proceeds_silently());
        assert!(RepoStatus::HostUnverified.proceeds_silently());
        assert!(!RepoStatus::Mismatch.proceeds_silently());
        assert!(!RepoStatus::NotInRepo.proceeds_silently());
    }

    #[test]
    fn display_names_the_host_only_when_the_hosts_differ() {
        let same = RepoDecision {
            status: RepoStatus::Mismatch,
            session: Some(r("github.com", "acme", "widgets")),
            current: Some(r("github.com", "other", "thing")),
        };
        assert_eq!(
            display_pair(&same),
            ("acme/widgets".to_string(), "other/thing".to_string())
        );

        let differ = RepoDecision {
            status: RepoStatus::Mismatch,
            session: Some(r("github.com", "acme", "widgets")),
            current: Some(r("ghe.corp", "acme", "widgets")),
        };
        assert_eq!(
            display_pair(&differ),
            (
                "github.com/acme/widgets".to_string(),
                "ghe.corp/acme/widgets".to_string()
            )
        );
    }

    #[test]
    fn decision_json_carries_owner_and_name_separately() {
        let d = RepoDecision {
            status: RepoStatus::Mismatch,
            session: Some(r("github.com", "eilonwy06", "claudecode-eclipse-ide")),
            current: Some(r("github.com", "other", "thing")),
        };
        let j: serde_json::Value = serde_json::from_str(&decision_json(&d)).unwrap();
        assert_eq!(j["status"], "mismatch");
        assert_eq!(j["proceed"], false);
        assert_eq!(j["sessionOwner"], "eilonwy06");
        assert_eq!(j["sessionName"], "claudecode-eclipse-ide");
        assert_eq!(j["sessionDisplay"], "eilonwy06/claudecode-eclipse-ide");
    }

    #[test]
    fn branch_names_that_git_would_reject_are_rejected_here() {
        assert!(is_valid_branch_name("dev"));
        assert!(is_valid_branch_name("feature/new-thing"));
        assert!(is_valid_branch_name("release-1.2.3"));

        // The one that actually matters: a leading dash becomes a git flag.
        assert!(!is_valid_branch_name("--upload-pack=evil"));
        assert!(!is_valid_branch_name("-x"));

        assert!(!is_valid_branch_name(""));
        assert!(!is_valid_branch_name("@"));
        assert!(!is_valid_branch_name("has space"));
        assert!(!is_valid_branch_name("a..b"));
        assert!(!is_valid_branch_name("a//b"));
        assert!(!is_valid_branch_name("a@{0}"));
        assert!(!is_valid_branch_name("tip."));
        assert!(!is_valid_branch_name("tip.lock"));
        assert!(!is_valid_branch_name("feature/.hidden"));
        assert!(!is_valid_branch_name("trailing/"));
        assert!(!is_valid_branch_name("ctrl\u{7}char"));
        assert!(!is_valid_branch_name("star*"));
        assert!(!is_valid_branch_name("colon:name"));
    }

    /// Guards the one path where a bad name would reach git as an argument.
    #[test]
    fn branch_lookups_refuse_an_invalid_name_without_running_git() {
        let root = std::env::temp_dir();
        assert!(!branch_exists_local(&root, "--upload-pack=evil"));
        assert!(!branch_exists_on_origin(&root, "--upload-pack=evil"));
    }
}

/// Exercises the real `git` subprocess path rather than the pure functions
/// above — the classifier is only as good as what `git remote get-url` hands
/// it, and that seam is where a quoting or working-directory mistake would
/// live. Builds a throwaway repo so it is portable and touches nothing.
#[cfg(test)]
mod git_tests {
    use super::*;

    struct TempRepo(std::path::PathBuf);

    impl Drop for TempRepo {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// `None` when git is unavailable, so the suite still passes on a machine
    /// without it instead of reporting a failure that is not ours.
    fn make_repo(tag: &str, origin: &str) -> Option<TempRepo> {
        let dir = std::env::temp_dir().join(format!("claude-teleport-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).ok()?;
        let repo = TempRepo(dir);
        git(&repo.0, &["init", "--quiet"])?;
        git(&repo.0, &["remote", "add", "origin", origin])?;
        Some(repo)
    }

    #[test]
    fn classifies_a_real_checkout_by_its_actual_remote() {
        let Some(repo) = make_repo("match", "https://github.com/acme/widgets.git") else {
            return; // no git on this machine
        };
        let root = repo.0.to_string_lossy().to_string();

        assert_eq!(
            remote_url(&repo.0, "origin").as_deref(),
            Some("https://github.com/acme/widgets.git")
        );

        let same = classify("https://github.com/acme/widgets.git", &root);
        assert_eq!(same.status, RepoStatus::Match, "same repo must match");

        // The ssh spelling of the same repo is still the same repo.
        let ssh = classify("git@github.com:acme/widgets.git", &root);
        assert_eq!(ssh.status, RepoStatus::Match, "ssh form must still match");

        let other = classify("https://github.com/other/thing.git", &root);
        assert_eq!(other.status, RepoStatus::Mismatch);
        assert_eq!(display_pair(&other).1, "acme/widgets");

        assert_eq!(classify("", &root).status, RepoStatus::NoRepoRequired);
    }

    #[test]
    fn a_folder_with_no_git_is_not_in_repo() {
        let dir = std::env::temp_dir().join(format!("claude-teleport-plain-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let d = classify("https://github.com/acme/widgets.git", &dir.to_string_lossy());
        assert_eq!(d.status, RepoStatus::NotInRepo);
        assert!(!d.status.proceeds_silently(), "must interrupt, not proceed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An empty root must not be answered from whatever directory the process
    /// happens to be sitting in — that would classify against the wrong repo.
    #[test]
    fn an_empty_root_never_borrows_the_process_directory() {
        assert!(remote_url(Path::new(""), "origin").is_none());
        assert_eq!(
            classify("https://github.com/acme/widgets.git", "").status,
            RepoStatus::NotInRepo
        );
    }

    #[test]
    fn branch_questions_answer_from_the_real_repo() {
        let Some(repo) = make_repo("branch", "https://github.com/acme/widgets.git") else {
            return;
        };
        git(&repo.0, &["config", "user.email", "t@example.com"]);
        git(&repo.0, &["config", "user.name", "t"]);
        std::fs::write(repo.0.join("a.txt"), "x").unwrap();
        git(&repo.0, &["add", "-A"]);
        git(&repo.0, &["commit", "--quiet", "-m", "init"]);

        let branch = current_branch(&repo.0).expect("a branch after the first commit");
        assert!(branch_exists_local(&repo.0, &branch), "{} should exist", branch);
        assert!(!branch_exists_local(&repo.0, "no-such-branch"));

        assert!(changed_files(&repo.0).is_empty(), "clean after commit");
        std::fs::write(repo.0.join("b.txt"), "y").unwrap();
        assert!(
            changed_files(&repo.0).iter().any(|f| f.ends_with("b.txt")),
            "an untracked file counts as a change"
        );
    }
}

/// The event→transcript conversion, which is where a wrong assumption would be
/// invisible: the wrong sort key reorders a conversation, and keeping protocol
/// frames would render handshake noise as if someone had said it.
#[cfg(test)]
mod transcript_tests {
    use super::*;
    use serde_json::json;

    fn ev(seq: &str, kind: &str, created: &str) -> serde_json::Value {
        json!({
            "sequence_num": seq,
            "created_at": created,
            "event_type": kind,
            "payload": { "type": kind, "uuid": format!("u{}", seq),
                         "message": {"role": "user", "content": format!("m{}", seq)} }
        })
    }

    #[test]
    fn keeps_only_the_four_types_that_carry_the_conversation() {
        // Counts taken from a real session: 34 control_request / 34
        // control_response / 6 rate_limit_event are protocol, not transcript.
        let events = vec![
            ev("1", "user", "2026-09-01T00:00:01Z"),
            ev("2", "control_request", "2026-09-01T00:00:02Z"),
            ev("3", "assistant", "2026-09-01T00:00:03Z"),
            ev("4", "control_response", "2026-09-01T00:00:04Z"),
            ev("5", "rate_limit_event", "2026-09-01T00:00:05Z"),
            ev("6", "system", "2026-09-01T00:00:06Z"),
            ev("7", "result", "2026-09-01T00:00:07Z"),
        ];
        let lines = to_transcript(&events, "local-1", "");
        let kinds: Vec<&str> = lines.iter().map(|l| l["type"].as_str().unwrap()).collect();
        assert_eq!(kinds, vec!["user", "assistant", "system", "result"]);
    }

    #[test]
    fn orders_numerically_not_lexically() {
        // The killer: sequence_num arrives as a STRING, so a plain sort puts
        // "10" before "9" and silently scrambles any conversation past nine.
        let events = vec![
            ev("10", "user", "2026-09-01T00:00:10Z"),
            ev("9", "user", "2026-09-01T00:00:09Z"),
            ev("100", "user", "2026-09-01T00:01:40Z"),
            ev("1", "user", "2026-09-01T00:00:01Z"),
        ];
        let lines = to_transcript(&events, "local-1", "");
        let order: Vec<&str> = lines
            .iter()
            .map(|l| l["message"]["content"].as_str().unwrap())
            .collect();
        assert_eq!(order, vec!["m1", "m9", "m10", "m100"]);
    }

    #[test]
    fn stamps_the_time_and_reassigns_the_session() {
        let events = vec![ev("1", "user", "2026-09-05T10:00:00Z")];
        let lines = to_transcript(&events, "local-42", "C:/ws");
        assert_eq!(lines[0]["timestamp"], "2026-09-05T10:00:00Z");
        assert_eq!(lines[0]["sessionId"], "local-42");
        assert_eq!(lines[0]["session_id"], "local-42");
        assert_eq!(lines[0]["cwd"], "C:/ws");
        // The payload's own uuid is preserved — the renderer keys off it.
        assert_eq!(lines[0]["uuid"], "u1");
    }

    #[test]
    fn survives_events_with_no_payload_or_no_sequence() {
        let events = vec![
            json!({"created_at": "2026-09-01T00:00:00Z"}),
            json!({"payload": {"type": "user"}, "created_at": "2026-09-01T00:00:01Z"}),
        ];
        let lines = to_transcript(&events, "l", "");
        assert_eq!(lines.len(), 1, "the payload-less event is skipped, not fatal");
    }

    #[test]
    fn a_remote_control_session_names_no_repo_and_no_branch() {
        // Verbatim shape from the live detail record: sources is empty and no
        // branch appears anywhere. This is the case that must NOT touch git.
        let detail = json!({
            "id": "cse_01Hasw", "title": "How remote control works",
            "tags": ["remote-control-sdk"],
            "config": {"sources": [], "model": "claude-opus-5"}
        });
        assert_eq!(session_repo_url(&detail), "");
        assert_eq!(session_branch(&detail), "");
        assert_eq!(
            classify(&session_repo_url(&detail), "").status,
            RepoStatus::NoRepoRequired
        );
    }

    #[test]
    fn finds_a_repo_and_branch_when_the_session_has_them() {
        let detail = json!({"config": {"sources": [
            {"type": "other", "url": "ignored"},
            {"type": "git_repository", "url": "https://github.com/acme/widgets.git",
             "branch": "feature/x"}
        ]}});
        assert_eq!(session_repo_url(&detail), "https://github.com/acme/widgets.git");
        assert_eq!(session_branch(&detail), "feature/x");
    }

    /// A branch name is server-supplied and ends up as a git argument.
    #[test]
    fn a_hostile_branch_name_is_dropped_at_the_boundary() {
        let detail = json!({"branch": "--upload-pack=touch /tmp/pwned"});
        assert_eq!(session_branch(&detail), "", "must not reach git");
        let out: serde_json::Value =
            serde_json::from_str(&checkout_branch("", "--upload-pack=evil")).unwrap();
        assert_eq!(out["ok"], false);
        assert_eq!(out["message"], "Invalid branch name.");
    }

    #[test]
    fn writes_and_reads_back_a_local_transcript() {
        // Point HOME at a temp dir so the real ~/.claude is never touched.
        let tmp = std::env::temp_dir().join(format!("claude-tp-write-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let prev_win = std::env::var("USERPROFILE").ok();
        let prev_unix = std::env::var("HOME").ok();
        std::env::set_var("USERPROFILE", &tmp);
        std::env::set_var("HOME", &tmp);

        let events = vec![ev("1", "user", "2026-09-01T00:00:01Z"), ev("2", "assistant", "2026-09-01T00:00:02Z")];
        let lines = to_transcript(&events, "pending", "C--tpws");
        let id = write_local_session("C--tpws", &lines).expect("should write");

        let path = tmp
            .join(".claude").join("projects")
            .join(crate::session::workspace_hash("C--tpws"))
            .join(format!("{}.jsonl", id));
        let body = std::fs::read_to_string(&path).expect("file should exist");
        assert_eq!(body.lines().count(), 2, "one json object per line");
        for l in body.lines() {
            serde_json::from_str::<serde_json::Value>(l).expect("each line is valid json");
        }
        // A uuid, not the remote id — teleporting twice must not collide.
        assert_ne!(id, "cse_01Hasw");
        assert_eq!(id.len(), 36, "uuid v4 with dashes");

        if let Some(v) = prev_win { std::env::set_var("USERPROFILE", v) } else { std::env::remove_var("USERPROFILE") }
        if let Some(v) = prev_unix { std::env::set_var("HOME", v) } else { std::env::remove_var("HOME") }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn the_boundary_marker_is_shaped_like_a_system_event() {
        let m = teleport_marker("local-1", "C:/ws");
        assert_eq!(m["type"], "system");
        assert_eq!(m["subtype"], "teleported_from_web");
        assert_eq!(m["sessionId"], "local-1");
        assert_eq!(m["cwd"], "C:/ws");
        let ts = m["timestamp"].as_str().unwrap();
        assert!(ts.ends_with('Z') && ts.len() == 24, "iso8601: {}", ts);
        // Must survive a jsonl round trip on one line.
        assert_eq!(m.to_string().lines().count(), 1);
    }

    #[test]
    fn the_timestamp_is_a_real_date() {
        let ts = now_iso8601();
        let year: i32 = ts[0..4].parse().unwrap();
        let month: u32 = ts[5..7].parse().unwrap();
        let day: u32 = ts[8..10].parse().unwrap();
        assert!(year >= 2024 && year < 2100, "{}", ts);
        assert!((1..=12).contains(&month), "{}", ts);
        assert!((1..=31).contains(&day), "{}", ts);
        assert_eq!(&ts[4..5], "-");
        assert_eq!(&ts[10..11], "T");
    }
}
