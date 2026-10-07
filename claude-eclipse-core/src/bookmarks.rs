//! Bookmarked replies: the replies of a conversation the user marked to find again,
//! shown beside it in the Claude Code view.
//!
//! A bookmark is a pointer, not a copy: the transcript line of the reply (its `uuid`)
//! and two times. The text is read back from the transcript when it is wanted, so a
//! bookmark never says anything the conversation does not.
//!
//! One file per conversation, `<dir>/<session id>.json`, in a directory the caller
//! names; a conversation with no bookmarks has no file. The record is the VS Code
//! extension's for the same feature.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// The longest id a bookmark or a conversation may have.
const MAX_ID_LEN: usize = 128;

/// The most replies whose text is read in one call.
const MAX_ASKED: usize = 1000;

/// One read-modify-write of a conversation's bookmarks at a time.
static STORE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Bookmark {
    /// The transcript line of the reply.
    uuid: String,
    /// When it was bookmarked, in ms since the epoch.
    #[serde(rename = "addedAt")]
    added_at: u64,
    /// When the reply was written, when that is known: what the list is ordered by
    /// and shows.
    #[serde(rename = "writtenAt", default, skip_serializing_if = "Option::is_none")]
    written_at: Option<u64>,
}

// ---------------------------------------------------------------------------
// A conversation's bookmarks
// ---------------------------------------------------------------------------

/// A conversation's bookmarks, oldest reply first, as a JSON array of
/// `{uuid, addedAt, writtenAt?}`.
pub fn list(dir: &str, session_id: &str) -> String {
    let bookmarks = file_for(dir, session_id).map(|path| read(&path)).unwrap_or_default();
    serde_json::to_string(&bookmarks).unwrap_or_else(|_| "[]".into())
}

/// Bookmarks the reply `uuid` of a conversation (`on`), or takes its bookmark away.
/// `written_at_ms` is when the reply was written, 0 or less when that is not known.
///
/// Returns `{"ok": <whether it was recorded>, "bookmarks": [<the list as it now is>]}`.
pub fn set(dir: &str, session_id: &str, uuid: &str, on: bool, written_at_ms: i64) -> String {
    set_at(dir, session_id, uuid, on, written_at_ms, now_ms())
}

/// [`set`] at a given time.
fn set_at(dir: &str, session_id: &str, uuid: &str, on: bool, written_at_ms: i64, now_ms: u64) -> String {
    let answer = |ok: bool, bookmarks: &[Bookmark]| serde_json::json!({ "ok": ok, "bookmarks": bookmarks }).to_string();
    let Some(path) = file_for(dir, session_id).filter(|_| plain_id(uuid)) else {
        return answer(false, &[]);
    };
    let _one_at_a_time = STORE_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let before = read(&path);
    let mut after: Vec<Bookmark> = before.iter().filter(|b| b.uuid != uuid).cloned().collect();
    if on {
        after.push(Bookmark {
            uuid: uuid.to_string(),
            added_at: now_ms,
            written_at: u64::try_from(written_at_ms).ok().filter(|at| *at > 0),
        });
    } else if after.len() == before.len() {
        // Not bookmarked to begin with: nothing to write.
        return answer(true, &before);
    }
    let after = ordered(after);
    if write(&path, &after, now_ms) {
        answer(true, &after)
    } else {
        // Not kept, so not shown as kept: the list is what is still on disk.
        answer(false, &before)
    }
}

/// Where a conversation's bookmarks are kept; None when `session_id` is not a plain name.
fn file_for(dir: &str, session_id: &str) -> Option<PathBuf> {
    if dir.is_empty() || !plain_id(session_id) {
        return None;
    }
    Some(Path::new(dir).join(format!("{session_id}.json")))
}

/// Whether an id can be a file name and a bookmark's key: not empty, not long, and
/// naming nothing outside the directory it is used in.
pub(crate) fn plain_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_ID_LEN && !id.contains(['/', '\\', ':']) && !id.contains("..")
}

/// The bookmarks in a file, in order; none when it is missing or cannot be read.
fn read(path: &Path) -> Vec<Bookmark> {
    let Some(file) = fs::read_to_string(path).ok().and_then(|json| serde_json::from_str::<serde_json::Value>(&json).ok()) else {
        return Vec::new();
    };
    // Entry by entry: one that cannot be read costs that bookmark, not the rest.
    let entries = file["bookmarks"].as_array().cloned().unwrap_or_default();
    ordered(entries.into_iter().filter_map(|entry| serde_json::from_value(entry).ok()).collect())
}

/// One bookmark per reply, oldest reply first. A reply's age is when it was written,
/// or when it was bookmarked if that is all that is known.
fn ordered(bookmarks: Vec<Bookmark>) -> Vec<Bookmark> {
    let mut seen = HashSet::new();
    let mut kept: Vec<Bookmark> =
        bookmarks.into_iter().filter(|b| plain_id(&b.uuid) && seen.insert(b.uuid.clone())).collect();
    kept.sort_by_key(|b| b.written_at.unwrap_or(b.added_at));
    kept
}

/// Writes a conversation's bookmarks whole; with none left, removes the file.
fn write(path: &Path, bookmarks: &[Bookmark], now_ms: u64) -> bool {
    if bookmarks.is_empty() {
        return fs::remove_file(path).is_ok() || !path.exists();
    }
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
        return false;
    };
    let json = serde_json::json!({ "updatedAt": now_ms, "bookmarks": bookmarks }).to_string();
    let _ = fs::create_dir_all(dir);
    let tmp = dir.join(format!("{}.tmp", name.to_string_lossy()));
    let written = fs::write(&tmp, json).is_ok() && fs::rename(&tmp, path).is_ok();
    if !written {
        let _ = fs::remove_file(&tmp);
    }
    written
}

// ---------------------------------------------------------------------------
// The text of a bookmarked reply
// ---------------------------------------------------------------------------

/// The text of replies of a conversation, read from its transcript: a JSON object of
/// `uuid → text`, with null for a reply the transcript does not hold. `uuids_json` is
/// a JSON array of at most 1000 ids; anything else answers `{}`.
pub fn texts(workspace_root: &str, session_id: &str, uuids_json: &str) -> String {
    let asked: Vec<String> = serde_json::from_str(uuids_json).unwrap_or_default();
    let path = crate::session::projects_dir(workspace_root)
        .filter(|_| plain_id(session_id))
        .map(|dir| dir.join(format!("{session_id}.jsonl")));
    serde_json::to_string(&texts_in(path.as_deref(), &asked)).unwrap_or_else(|_| "{}".into())
}

/// [`texts`] over a given transcript file.
fn texts_in(transcript: Option<&Path>, asked: &[String]) -> BTreeMap<String, Option<String>> {
    if asked.len() > MAX_ASKED || !asked.iter().all(|uuid| plain_id(uuid)) {
        return BTreeMap::new();
    }
    let mut found: BTreeMap<String, Option<String>> = asked.iter().map(|uuid| (uuid.clone(), None)).collect();
    let Some(raw) = transcript.and_then(|path| fs::read_to_string(path).ok()) else {
        return found;
    };
    let mut missing = asked.len();
    for line in raw.lines() {
        if missing == 0 {
            break;
        }
        // Cheap reject first: parsing every line of a long transcript is not free.
        if !line.contains("\"assistant\"") || !asked.iter().any(|uuid| line.contains(uuid.as_str())) {
            continue;
        }
        let Ok(event) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(slot) = event["uuid"].as_str().and_then(|uuid| found.get_mut(uuid)) else {
            continue;
        };
        if slot.is_none() {
            if let Some(text) = reply_text(&event) {
                *slot = Some(text);
                missing -= 1;
            }
        }
    }
    found
}

/// What an assistant line says: its text blocks, a blank line between them. None for
/// a line that is not a reply or says nothing (a tool call, a thinking block).
fn reply_text(event: &serde_json::Value) -> Option<String> {
    if event["type"].as_str() != Some("assistant") {
        return None;
    }
    let said: Vec<&str> = event["message"]["content"]
        .as_array()?
        .iter()
        .filter(|block| block["type"].as_str() == Some("text"))
        .filter_map(|block| block["text"].as_str())
        .collect();
    if said.is_empty() {
        None
    } else {
        Some(said.join("\n\n"))
    }
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_800_000_000_000;
    const SESSION: &str = "0b9c3a7e-1111-4222-8333-444455556666";

    fn store() -> (tempfile::TempDir, String) {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("session-bookmarks").to_str().unwrap().to_string();
        (tmp, dir)
    }

    /// `set_at`'s answer as `(ok, uuids in order)`.
    fn answer(json: &str) -> (bool, Vec<String>) {
        let out: serde_json::Value = serde_json::from_str(json).expect("set answers in JSON");
        let uuids = out["bookmarks"].as_array().expect("a bookmarks array").iter().map(|b| b["uuid"].as_str().unwrap().to_string()).collect();
        (out["ok"].as_bool().expect("an ok flag"), uuids)
    }

    fn uuids(dir: &str) -> Vec<String> {
        let out: serde_json::Value = serde_json::from_str(&list(dir, SESSION)).expect("list answers in JSON");
        out.as_array().unwrap().iter().map(|b| b["uuid"].as_str().unwrap().to_string()).collect()
    }

    fn ids(of: &[&str]) -> Vec<String> {
        of.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_conversation_with_no_bookmarks_has_an_empty_list_and_no_file() {
        let (tmp, dir) = store();

        assert_eq!(list(&dir, SESSION), "[]");
        assert!(!tmp.path().join("session-bookmarks").exists());
    }

    #[test]
    fn bookmarking_a_reply_records_it_with_both_times() {
        let (_tmp, dir) = store();

        let out = set_at(&dir, SESSION, "reply-1", true, 1_700_000_000_000, NOW);

        assert_eq!(answer(&out), (true, ids(&["reply-1"])));
        let stored: serde_json::Value = serde_json::from_str(&list(&dir, SESSION)).unwrap();
        assert_eq!(stored, serde_json::json!([{ "uuid": "reply-1", "addedAt": NOW, "writtenAt": 1_700_000_000_000u64 }]));
    }

    #[test]
    fn the_file_is_the_conversations_own_and_says_when_it_changed() {
        let (tmp, dir) = store();
        set_at(&dir, SESSION, "reply-1", true, 5, NOW);

        let path = tmp.path().join("session-bookmarks").join(format!("{SESSION}.json"));
        let file: serde_json::Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(file["updatedAt"], NOW);
        assert_eq!(file["bookmarks"].as_array().unwrap().len(), 1);
        assert_eq!(list(&dir, "another-session"), "[]");
    }

    #[test]
    fn a_reply_whose_time_is_not_known_is_kept_without_one() {
        let (_tmp, dir) = store();

        set_at(&dir, SESSION, "no-time", true, 0, NOW);
        set_at(&dir, SESSION, "negative", true, -5, NOW + 1);

        let stored: serde_json::Value = serde_json::from_str(&list(&dir, SESSION)).unwrap();
        assert_eq!(stored, serde_json::json!([
            { "uuid": "no-time", "addedAt": NOW }, { "uuid": "negative", "addedAt": NOW + 1 }
        ]));
    }

    #[test]
    fn the_list_runs_from_the_oldest_reply_to_the_newest_whatever_order_they_were_bookmarked_in() {
        let (_tmp, dir) = store();

        set_at(&dir, SESSION, "third", true, 3_000, NOW);
        set_at(&dir, SESSION, "first", true, 1_000, NOW + 1);
        let out = set_at(&dir, SESSION, "second", true, 2_000, NOW + 2);

        assert_eq!(answer(&out), (true, ids(&["first", "second", "third"])));
        assert_eq!(uuids(&dir), ids(&["first", "second", "third"]));
    }

    #[test]
    fn a_reply_with_no_written_time_is_placed_by_when_it_was_bookmarked() {
        let unordered = vec![
            Bookmark { uuid: "written-late".into(), added_at: 10, written_at: Some(900) },
            Bookmark { uuid: "added-500".into(), added_at: 500, written_at: None },
            Bookmark { uuid: "written-early".into(), added_at: 999, written_at: Some(100) },
        ];

        let order: Vec<String> = ordered(unordered).into_iter().map(|b| b.uuid).collect();
        assert_eq!(order, ids(&["written-early", "added-500", "written-late"]));
    }

    #[test]
    fn bookmarking_the_same_reply_again_keeps_one_bookmark() {
        let (_tmp, dir) = store();
        set_at(&dir, SESSION, "reply-1", true, 1_000, NOW);

        let out = set_at(&dir, SESSION, "reply-1", true, 1_000, NOW + 50);

        assert_eq!(answer(&out), (true, ids(&["reply-1"])));
        let stored: serde_json::Value = serde_json::from_str(&list(&dir, SESSION)).unwrap();
        assert_eq!(stored[0]["addedAt"], NOW + 50);
    }

    #[test]
    fn removing_a_bookmark_takes_only_that_one_out() {
        let (_tmp, dir) = store();
        set_at(&dir, SESSION, "keep", true, 1_000, NOW);
        set_at(&dir, SESSION, "drop", true, 2_000, NOW);

        let out = set_at(&dir, SESSION, "drop", false, 0, NOW + 1);

        assert_eq!(answer(&out), (true, ids(&["keep"])));
        assert_eq!(uuids(&dir), ids(&["keep"]));
    }

    #[test]
    fn removing_the_last_bookmark_removes_the_file() {
        let (tmp, dir) = store();
        set_at(&dir, SESSION, "only", true, 1_000, NOW);
        let path = tmp.path().join("session-bookmarks").join(format!("{SESSION}.json"));
        assert!(path.exists());

        let out = set_at(&dir, SESSION, "only", false, 0, NOW + 1);

        assert_eq!(answer(&out), (true, ids(&[])));
        assert!(!path.exists());
    }

    #[test]
    fn removing_a_bookmark_that_is_not_there_changes_nothing() {
        let (tmp, dir) = store();
        set_at(&dir, SESSION, "keep", true, 1_000, NOW);
        let path = tmp.path().join("session-bookmarks").join(format!("{SESSION}.json"));
        let before = fs::read_to_string(&path).unwrap();

        let out = set_at(&dir, SESSION, "never-was", false, 0, NOW + 9);

        assert_eq!(answer(&out), (true, ids(&["keep"])));
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
    }

    #[test]
    fn an_id_that_is_not_a_plain_name_is_refused() {
        let long = "x".repeat(MAX_ID_LEN + 1);
        for bad in ["", "..", "a/b", "a\\b", "../up", "c:evil", long.as_str()] {
            assert!(!plain_id(bad), "{bad:?}");
        }
        for good in [SESSION, "reply-1", "a.b", &"x".repeat(MAX_ID_LEN)] {
            assert!(plain_id(good), "{good:?}");
        }
    }

    #[test]
    fn nothing_is_recorded_for_a_conversation_or_a_reply_that_cannot_be_named() {
        let (tmp, dir) = store();

        assert_eq!(answer(&set_at(&dir, "../escape", "reply-1", true, 1, NOW)), (false, ids(&[])));
        assert_eq!(answer(&set_at(&dir, SESSION, "", true, 1, NOW)), (false, ids(&[])));
        assert_eq!(answer(&set_at(&dir, SESSION, "a/b", true, 1, NOW)), (false, ids(&[])));
        assert_eq!(answer(&set_at("", SESSION, "reply-1", true, 1, NOW)), (false, ids(&[])));

        assert!(!tmp.path().join("session-bookmarks").exists());
        assert!(!tmp.path().join("escape.json").exists());
        assert_eq!(list(&dir, "../escape"), "[]");
    }

    #[test]
    fn a_file_is_read_for_what_it_holds_and_the_rest_is_skipped() {
        let (tmp, dir) = store();
        let folder = tmp.path().join("session-bookmarks");
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join(format!("{SESSION}.json")), r#"{"updatedAt": 5, "bookmarks": [
            {"uuid": "late", "addedAt": 1, "writtenAt": 900},
            "not a bookmark",
            {"uuid": "", "addedAt": 2},
            {"addedAt": 3},
            {"uuid": "late", "addedAt": 4, "writtenAt": 1},
            {"uuid": "early", "addedAt": 5, "writtenAt": 100, "somethingNewer": true}
        ]}"#).unwrap();

        assert_eq!(uuids(&dir), ids(&["early", "late"]));
    }

    #[test]
    fn a_file_that_is_not_a_bookmark_file_is_an_empty_list() {
        let (tmp, dir) = store();
        let folder = tmp.path().join("session-bookmarks");
        fs::create_dir_all(&folder).unwrap();
        for garbage in ["", "{ not json", "[1, 2]", r#"{"bookmarks": "none"}"#] {
            fs::write(folder.join(format!("{SESSION}.json")), garbage).unwrap();
            assert_eq!(list(&dir, SESSION), "[]", "{garbage:?}");
        }
    }

    #[test]
    fn a_bookmark_that_cannot_be_written_is_not_reported_as_kept() {
        let (tmp, dir) = store();
        // A directory with something in it where the file should be: it cannot be replaced.
        let in_the_way = tmp.path().join("session-bookmarks").join(format!("{SESSION}.json"));
        fs::create_dir_all(&in_the_way).unwrap();
        fs::write(in_the_way.join("occupied"), "").unwrap();

        let out = set_at(&dir, SESSION, "reply-1", true, 1_000, NOW);

        assert_eq!(answer(&out), (false, ids(&[])));
        assert!(!tmp.path().join("session-bookmarks").join(format!("{SESSION}.json.tmp")).exists());
    }

    fn transcript(lines: &[&str]) -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("session.jsonl");
        fs::write(&path, lines.join("\n") + "\n").unwrap();
        (tmp, path)
    }

    const REPLY: &str = r#"{"type":"assistant","uuid":"a-1","message":{"content":[{"type":"text","text":"Here is the answer."}]},"timestamp":"2026-10-01T10:00:00.000Z"}"#;
    const TWO_BLOCKS: &str = r#"{"type":"assistant","uuid":"a-2","message":{"content":[{"type":"text","text":"First part."},{"type":"tool_use","id":"t1","name":"Read","input":{}},{"type":"text","text":"Second part."}]}}"#;
    const TOOL_ONLY: &str = r#"{"type":"assistant","uuid":"a-3","message":{"content":[{"type":"tool_use","id":"t2","name":"Bash","input":{}}]}}"#;
    const USER_QUOTING: &str = r#"{"type":"user","uuid":"u-1","message":{"role":"user","content":"what did a-1 mean, and \"uuid\":\"a-9\"?"}}"#;

    #[test]
    fn a_replys_text_is_its_text_blocks_with_a_blank_line_between() {
        let one: serde_json::Value = serde_json::from_str(REPLY).unwrap();
        let two: serde_json::Value = serde_json::from_str(TWO_BLOCKS).unwrap();

        assert_eq!(reply_text(&one).as_deref(), Some("Here is the answer."));
        assert_eq!(reply_text(&two).as_deref(), Some("First part.\n\nSecond part."));
    }

    #[test]
    fn a_line_that_says_nothing_or_is_not_claudes_has_no_reply_text() {
        for line in [TOOL_ONLY, USER_QUOTING, r#"{"type":"assistant","uuid":"x","message":{"content":"a string"}}"#, r#"{"type":"assistant","uuid":"y"}"#] {
            let event: serde_json::Value = serde_json::from_str(line).unwrap();
            assert_eq!(reply_text(&event), None, "{line}");
        }
    }

    #[test]
    fn the_text_of_each_reply_asked_for_is_read_from_the_transcript() {
        let (_tmp, path) = transcript(&[USER_QUOTING, REPLY, TOOL_ONLY, TWO_BLOCKS]);

        let found = texts_in(Some(&path), &ids(&["a-2", "a-1"]));

        assert_eq!(found.get("a-1"), Some(&Some("Here is the answer.".to_string())));
        assert_eq!(found.get("a-2"), Some(&Some("First part.\n\nSecond part.".to_string())));
        assert_eq!(found.len(), 2);
    }

    #[test]
    fn a_reply_the_transcript_does_not_hold_has_no_text() {
        let (_tmp, path) = transcript(&[USER_QUOTING, REPLY, TOOL_ONLY]);

        // a-9 is only quoted by a user line, a-3 is a tool call, u-1 is not Claude's.
        let found = texts_in(Some(&path), &ids(&["a-9", "a-3", "u-1", "gone"]));

        assert_eq!(found.values().filter(|text| text.is_some()).count(), 0);
        assert_eq!(found.len(), 4);
    }

    #[test]
    fn without_a_transcript_every_reply_asked_for_has_no_text() {
        let tmp = tempfile::tempdir().unwrap();

        for missing in [None, Some(tmp.path().join("no-such.jsonl"))] {
            let found = texts_in(missing.as_deref(), &ids(&["a-1"]));
            assert_eq!(found.get("a-1"), Some(&None));
        }
    }

    #[test]
    fn asking_for_too_many_or_for_ids_that_are_not_plain_reads_nothing() {
        let (_tmp, path) = transcript(&[REPLY]);
        let too_many: Vec<String> = (0..=MAX_ASKED).map(|i| format!("a-{i}")).collect();

        assert!(texts_in(Some(&path), &too_many).is_empty());
        assert!(texts_in(Some(&path), &ids(&["a-1", "../x"])).is_empty());
        assert!(texts_in(Some(&path), &ids(&[])).is_empty());
    }

    #[test]
    fn the_callers_ids_arrive_as_json_and_a_list_that_is_not_one_asks_for_nothing() {
        assert_eq!(texts("", SESSION, "not json"), "{}");
        assert_eq!(texts("", SESSION, "[]"), "{}");
        assert_eq!(texts("", "../x", "[\"a-1\"]"), "{\"a-1\":null}");
    }
}
