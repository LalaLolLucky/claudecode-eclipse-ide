//! The file list behind the composer's `@` mentions.
//!
//! Follows the VS Code extension: every file under the conversation's working
//! folder (`rg --files --follow --hidden`, so .gitignore is honored and dotfiles
//! are not skipped), minus the editor's default excludes, with a folder row for
//! every folder that holds a listed file. The list is asked for again on each
//! keystroke, so a walk is cached per folder for a few seconds.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

const CACHE_TTL: Duration = Duration::from_secs(10);
/// Stops a walk of an enormous tree from holding the whole disk in memory.
const MAX_FILES: usize = 200_000;
/// Rows returned for a typed query — the extension's own limit.
const QUERY_LIMIT: usize = 100;
/// Rows returned for a bare `@`, which lists everything.
const BROWSE_LIMIT: usize = 5_000;
/// VS Code's default `files.exclude`, which it passes to ripgrep.
const EXCLUDED_NAMES: &[&str] = &[".git", ".svn", ".hg", "CVS", ".DS_Store", "Thumbs.db"];

struct Entry {
    /// Relative to the root, `/`-separated; a folder ends in `/`.
    path: String,
    name: String,
    dir: bool,
}

/// The `@` list for `query` under `root`, as a JSON array of
/// `{"path","name","type":"file"|"directory"}`. An empty query lists everything.
pub fn list_files_json(root: &str, query: &str) -> String {
    if root.trim().is_empty() {
        return "[]".to_string();
    }
    let entries = cached_entries(root);
    let rows: Vec<serde_json::Value> = if query.is_empty() {
        entries.iter().take(BROWSE_LIMIT).map(row).collect()
    } else {
        rank(&entries, query).into_iter().map(row).collect()
    };
    serde_json::to_string(&rows).unwrap_or_else(|_| "[]".to_string())
}

/// The `@` list with browser tabs in it, in the extension's order: the files, then
/// the tabs — except while the typed word could still become `browser:` ("b",
/// "bro"), when the tabs lead. Both inputs are JSON row arrays; bad input counts
/// as no rows.
pub fn with_browser_rows(files_json: &str, tabs_json: &str, query: &str) -> String {
    let parse = |s: &str| serde_json::from_str::<Vec<serde_json::Value>>(s).unwrap_or_default();
    let (files, tabs) = (parse(files_json), parse(tabs_json));
    let q = query.to_lowercase();
    let tabs_first = !q.is_empty() && "browser:".starts_with(&q);
    let rows: Vec<serde_json::Value> = if tabs_first {
        tabs.into_iter().chain(files).collect()
    } else {
        files.into_iter().chain(tabs).collect()
    };
    serde_json::to_string(&rows).unwrap_or_else(|_| "[]".to_string())
}

fn row(e: &Entry) -> serde_json::Value {
    serde_json::json!({
        "path": e.path,
        "name": e.name,
        "type": if e.dir { "directory" } else { "file" },
    })
}

fn cache() -> &'static Mutex<HashMap<String, (Instant, Arc<Vec<Entry>>)>> {
    static CACHE: OnceLock<Mutex<HashMap<String, (Instant, Arc<Vec<Entry>>)>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cached_entries(root: &str) -> Arc<Vec<Entry>> {
    if let Some((at, entries)) = cache().lock().unwrap().get(root) {
        if at.elapsed() < CACHE_TTL {
            return entries.clone();
        }
    }
    // Walked outside the lock: another folder's lookup must not wait on this one.
    let fresh = Arc::new(entries_from_files(walk(root)));
    cache().lock().unwrap().insert(root.to_string(), (Instant::now(), fresh.clone()));
    fresh
}

fn walk(root: &str) -> Vec<String> {
    let base = std::path::Path::new(root);
    let walker = ignore::WalkBuilder::new(base)
        .hidden(false)
        .follow_links(true)
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !EXCLUDED_NAMES.iter().any(|x| *x == name)
        })
        .build();
    let mut files = Vec::new();
    for dent in walker.flatten() {
        if !dent.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        let Ok(rel) = dent.path().strip_prefix(base) else { continue };
        let rel = rel.to_string_lossy().replace('\\', "/");
        if rel.is_empty() {
            continue;
        }
        files.push(rel);
        if files.len() >= MAX_FILES {
            break;
        }
    }
    files
}

/// Adds a row for every folder above a file, then orders the lot as a tree:
/// a folder comes right before its contents, and inside a folder its subfolders
/// come before its files.
fn entries_from_files(files: Vec<String>) -> Vec<Entry> {
    let mut dirs: BTreeSet<String> = BTreeSet::new();
    for f in &files {
        let mut from = 0;
        while let Some(p) = f[from..].find('/') {
            let end = from + p;
            dirs.insert(f[..end].to_string());
            from = end + 1;
        }
    }
    let mut out: Vec<Entry> = dirs
        .into_iter()
        .map(|d| Entry { name: basename(&d).to_string(), path: format!("{d}/"), dir: true })
        .chain(files.into_iter().map(|f| Entry { name: basename(&f).to_string(), path: f, dir: false }))
        .collect();
    out.sort_by_cached_key(|e| {
        let segs: Vec<&str> = e.path.trim_end_matches('/').split('/').collect();
        let last = segs.len() - 1;
        segs.iter()
            .enumerate()
            .map(|(i, s)| (!e.dir && i == last, s.to_lowercase()))
            .collect::<Vec<_>>()
    });
    out
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Best matches for `query`, best first, at most [`QUERY_LIMIT`]. A match on the
/// file name outranks the same match on the path, and paths containing "test"
/// lose ties — the weighting the extension gives its fuzzy search. A query with a
/// folder part ("src/ma") only looks inside that folder.
fn rank<'a>(entries: &'a [Entry], query: &str) -> Vec<&'a Entry> {
    let q = query.replace('\\', "/").to_lowercase();
    let folder = match q.rfind('/') {
        Some(i) if i > 2 => Some(&q[..i]),
        _ => None,
    };
    let mut scored: Vec<(u32, u8, usize, &Entry)> = Vec::new();
    for (idx, e) in entries.iter().enumerate() {
        let path = e.path.to_lowercase();
        if let Some(f) = folder {
            if !path.starts_with(f) {
                continue;
            }
        }
        let name = e.name.to_lowercase();
        let score = match (field_score(&name, &q), field_score(&path, &q)) {
            (Some(n), Some(p)) => n.min(p + 0.1),
            (Some(n), None) => n,
            (None, Some(p)) => p + 0.1,
            (None, None) => continue,
        };
        if score > 0.5 {
            continue;
        }
        let bucket = (score / 0.05).floor() as u32;
        let test_penalty = u8::from(path.contains("test"));
        scored.push((bucket, test_penalty, idx, e));
    }
    scored.sort_by_key(|(bucket, test, idx, _)| (*bucket, *test, *idx));
    scored.into_iter().take(QUERY_LIMIT).map(|s| s.3).collect()
}

/// 0.0 is an exact match; `None` is no match. Substrings beat scattered letters,
/// and a substring nearer the start beats one further in.
fn field_score(text: &str, q: &str) -> Option<f64> {
    if q.is_empty() || text == q {
        return Some(0.0);
    }
    let len = text.len().max(1) as f64;
    if let Some(pos) = text.find(q) {
        let base = if pos == 0 { 0.05 } else { 0.1 };
        return Some(base + 0.1 * (pos as f64 / len));
    }
    let mut chars = text.char_indices();
    let (mut first, mut last) = (None, 0usize);
    for qc in q.chars() {
        loop {
            match chars.next() {
                Some((i, c)) if c == qc => {
                    first.get_or_insert(i);
                    last = i;
                    break;
                }
                Some(_) => continue,
                None => return None,
            }
        }
    }
    let span = last - first.unwrap_or(0) + 1;
    let spread = span.saturating_sub(q.len()) as f64 / len;
    let score = 0.3 + 0.2 * spread;
    (score <= 0.5).then_some(score)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(entries: &[Entry]) -> Vec<&str> {
        entries.iter().map(|e| e.path.as_str()).collect()
    }

    #[test]
    fn browser_rows_follow_files_unless_the_word_could_become_browser() {
        let files = r#"[{"path":"a.txt","name":"a.txt","type":"file"}]"#;
        let tabs = r#"[{"path":"browser:new_tab","name":"browser:new_tab","type":"browser"}]"#;
        let order = |q: &str| -> Vec<String> {
            serde_json::from_str::<Vec<serde_json::Value>>(&with_browser_rows(files, tabs, q))
                .unwrap()
                .iter()
                .map(|r| r["type"].as_str().unwrap().to_string())
                .collect()
        };
        assert_eq!(order(""), ["file", "browser"]);
        assert_eq!(order("a"), ["file", "browser"]);
        assert_eq!(order("b"), ["browser", "file"]);
        assert_eq!(order("BRO"), ["browser", "file"]);
        assert_eq!(order("browser:"), ["browser", "file"]);
        assert_eq!(order("browsers"), ["file", "browser"]);
        assert_eq!(with_browser_rows(files, "not json", ""), with_browser_rows(files, "[]", ""));
    }

    #[test]
    fn folders_lead_their_contents_and_subfolders_lead_files() {
        let files = ["b.txt", "a/z.txt", "a/b/c.txt", "A2/x.txt"].map(String::from).to_vec();
        let entries = entries_from_files(files);
        assert_eq!(
            paths(&entries),
            ["a/", "a/b/", "a/b/c.txt", "a/z.txt", "A2/", "A2/x.txt", "b.txt"]
        );
        assert!(entries[0].dir && !entries[2].dir);
        assert_eq!(entries[1].name, "b");
    }

    #[test]
    fn folder_part_of_a_query_scopes_the_search() {
        let files = ["src/main.rs", "src/lib.rs", "docs/main.md"].map(String::from).to_vec();
        let entries = entries_from_files(files);
        let got: Vec<&str> = rank(&entries, "src/").iter().map(|e| e.path.as_str()).collect();
        assert_eq!(got, ["src/", "src/lib.rs", "src/main.rs"]);
    }

    #[test]
    fn name_match_outranks_path_match_and_scattered_letters_still_match() {
        let files = ["main/other.rs", "x/main.rs", "m_a_i_n.txt"].map(String::from).to_vec();
        let entries = entries_from_files(files);
        let got: Vec<&str> = rank(&entries, "main").iter().map(|e| e.path.as_str()).collect();
        assert_eq!(got[0], "main/");
        assert!(got.contains(&"x/main.rs"));
        assert!(got.contains(&"m_a_i_n.txt"));
        assert!(rank(&entries, "zzz").is_empty());
    }

    #[test]
    fn walk_keeps_dotfiles_and_skips_vcs_folders() {
        let root = std::env::temp_dir().join("claude-eclipse-mentions-walk");
        let _ = std::fs::remove_dir_all(&root);
        for p in [".git/config", ".hidden/h.txt", "sub/file.txt"] {
            let f = root.join(p);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, "x").unwrap();
        }
        let mut files = walk(&root.to_string_lossy());
        let _ = std::fs::remove_dir_all(&root);
        files.sort();
        assert_eq!(files, [".hidden/h.txt", "sub/file.txt"]);
    }
}
