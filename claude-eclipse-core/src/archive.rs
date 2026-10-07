//! The Claude Code view's archive: which saved conversations its history lists under
//! "Archived sessions", and the sweep that files inactive ones there.
//!
//! The CLI has no notion of an archived conversation, so this is the plug-in's own
//! record, kept in a file the caller names. Nothing in a transcript is touched:
//! archiving only changes where the history lists a conversation.
//!
//! The rules are the VS Code extension's for its `claudeCode.archiveInactiveSessions`
//! setting, so the setting means here what it means there.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const DAY_MS: u64 = 86_400_000;

/// The periods the preference offers, in days. Any other stored value means never.
const PERIODS: [u64; 4] = [1, 2, 7, 14];

/// How long an unarchive is remembered: the longest period. Past that it can no longer
/// keep a conversation out of the archive, so there is nothing left to remember.
const UNARCHIVE_MEMORY_DAYS: u64 = 14;

/// One read-modify-write of the store at a time: the history is loaded off the UI
/// thread while a click on a row's button arrives on it.
static STORE_LOCK: Mutex<()> = Mutex::new(());

/// What is kept on disk.
#[derive(Default, Serialize, Deserialize)]
struct Store {
    #[serde(default)]
    archived: Vec<String>,
    /// When a conversation was last taken out of the archive, in ms since the epoch.
    #[serde(default, rename = "unarchivedAt")]
    unarchived_at: BTreeMap<String, u64>,
}

/// A listed conversation and when its transcript was last written, 0 when that is
/// not known.
struct Listed {
    id: String,
    modified_ms: u64,
}

// ---------------------------------------------------------------------------
// The history list with the archive applied
// ---------------------------------------------------------------------------

/// The history list as the panel shows it: `sessions_json` (what
/// [`crate::session::list_sessions`] gave for `workspace_root`) with an `archived` flag
/// on every row, after archiving the rows that have been inactive for `days`.
///
/// Returns `{"sessions": [...], "archivedNow": [...]}`, the second being the ids this
/// call archived — what the caller tells the user about. `in_use_json` is a JSON array
/// of the ids open in a tab, which are never archived by the sweep.
pub fn apply(store_path: &str, workspace_root: &str, sessions_json: &str, days: i64, in_use_json: &str) -> String {
    let in_use: BTreeSet<String> = serde_json::from_str(in_use_json).unwrap_or_default();
    let dir = crate::session::projects_dir(workspace_root);
    apply_in(Path::new(store_path), dir.as_deref(), sessions_json, period(days), &in_use, now_ms())
}

/// [`apply`] over a given transcript directory, period and clock.
fn apply_in(
    store_path: &Path,
    dir: Option<&Path>,
    sessions_json: &str,
    days: u64,
    in_use: &BTreeSet<String>,
    now_ms: u64,
) -> String {
    let mut rows: Vec<serde_json::Value> = serde_json::from_str(sessions_json).unwrap_or_default();
    let listed: Vec<Listed> = rows
        .iter()
        .filter_map(|row| row["sessionId"].as_str())
        .map(|id| Listed { id: id.to_string(), modified_ms: dir.map_or(0, |d| modified_ms(d, id)) })
        .collect();

    let _one_at_a_time = STORE_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut store = read_store(store_path);
    let mut archived_now = inactive(&listed, days, now_ms, in_use, &store);
    if !archived_now.is_empty() {
        let before = store.archived.len();
        store.archived.extend(archived_now.iter().cloned());
        if !write_store(store_path, &store) {
            // Not recorded, so not archived: the list must not show what the next
            // load would undo.
            store.archived.truncate(before);
            archived_now.clear();
        }
    }

    let archived: BTreeSet<&str> = store.archived.iter().map(String::as_str).collect();
    for row in rows.iter_mut() {
        let is_archived = row["sessionId"].as_str().is_some_and(|id| archived.contains(id));
        if let Some(fields) = row.as_object_mut() {
            fields.insert("archived".into(), is_archived.into());
        }
    }
    serde_json::json!({ "sessions": rows, "archivedNow": archived_now }).to_string()
}

/// The period in days for the preference's stored value, 0 for never.
fn period(days: i64) -> u64 {
    PERIODS.iter().copied().find(|offered| i64::try_from(*offered) == Ok(days)).unwrap_or(0)
}

/// The conversations a sweep archives, in the order listed: those not archived
/// already, not in use, and neither written nor taken out of the archive within the
/// last `days`.
fn inactive(sessions: &[Listed], days: u64, now_ms: u64, in_use: &BTreeSet<String>, store: &Store) -> Vec<String> {
    if days == 0 {
        return Vec::new();
    }
    let cutoff = now_ms.saturating_sub(days * DAY_MS);
    let archived: BTreeSet<&str> = store.archived.iter().map(String::as_str).collect();
    sessions
        .iter()
        .filter(|s| !archived.contains(s.id.as_str()))
        // A time of 0 is "not known", and an unknown age is not an old one.
        .filter(|s| s.modified_ms > 0)
        .filter(|s| s.modified_ms.max(store.unarchived_at.get(&s.id).copied().unwrap_or(0)) <= cutoff)
        .filter(|s| !in_use.contains(&s.id))
        .map(|s| s.id.clone())
        .collect()
}

/// When a conversation's transcript was last written, in ms since the epoch; 0 when
/// there is no such file, or `id` is not a plain file name.
fn modified_ms(dir: &Path, id: &str) -> u64 {
    // The same test as session::delete_session: an id must not name a file elsewhere.
    if id.is_empty() || id.contains('/') || id.contains('\\') || id.contains("..") {
        return 0;
    }
    fs::metadata(dir.join(format!("{id}.jsonl")))
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |since| since.as_millis() as u64)
}

// ---------------------------------------------------------------------------
// Archiving and unarchiving by hand
// ---------------------------------------------------------------------------

/// Archives or unarchives the conversations of `ids_json` (a JSON array of ids).
/// Returns whether it was recorded.
pub fn set(store_path: &str, ids_json: &str, archived: bool) -> bool {
    let ids: Vec<String> = serde_json::from_str(ids_json).unwrap_or_default();
    set_at(store_path, &ids, archived, now_ms())
}

/// [`set`] at a given time.
fn set_at(store_path: &str, ids: &[String], archived: bool, now_ms: u64) -> bool {
    if store_path.is_empty() {
        return false;
    }
    let path = Path::new(store_path);
    let _one_at_a_time = STORE_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut store = read_store(path);
    if archived {
        for id in ids {
            if !store.archived.contains(id) {
                store.archived.push(id.clone());
            }
        }
    } else {
        // Remembered so the sweep does not put it straight back: see `inactive`.
        let oldest_kept = now_ms.saturating_sub(UNARCHIVE_MEMORY_DAYS * DAY_MS);
        store.unarchived_at.retain(|_, at| *at > oldest_kept);
        for id in ids {
            store.unarchived_at.insert(id.clone(), now_ms);
        }
        store.archived.retain(|id| !ids.contains(id));
    }
    write_store(path, &store)
}

// ---------------------------------------------------------------------------
// The store
// ---------------------------------------------------------------------------

/// The store as it is on disk; empty when the file is missing or cannot be read as one.
fn read_store(path: &Path) -> Store {
    fs::read_to_string(path).ok().and_then(|json| serde_json::from_str(&json).ok()).unwrap_or_default()
}

/// Writes the store whole, through a temporary file so a reader never sees half of it.
fn write_store(path: &Path, store: &Store) -> bool {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
        return false;
    };
    let Ok(json) = serde_json::to_string(store) else {
        return false;
    };
    let _ = fs::create_dir_all(dir);
    let tmp = dir.join(format!("{}.tmp", name.to_string_lossy()));
    let written = fs::write(&tmp, json).is_ok() && fs::rename(&tmp, path).is_ok();
    if !written {
        let _ = fs::remove_file(&tmp);
    }
    written
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
    use std::time::Duration;

    /// 2027-01-15, far enough from the epoch that a whole period fits below it.
    const NOW: u64 = 1_800_000_000_000;
    const WEEK: u64 = 7;

    fn listed(id: &str, modified_ms: u64) -> Listed {
        Listed { id: id.to_string(), modified_ms }
    }

    fn ids(of: &[&str]) -> Vec<String> {
        of.iter().map(|s| s.to_string()).collect()
    }

    fn set_of(of: &[&str]) -> BTreeSet<String> {
        of.iter().map(|s| s.to_string()).collect()
    }

    fn store(archived: &[&str], unarchived_at: &[(&str, u64)]) -> Store {
        Store {
            archived: ids(archived),
            unarchived_at: unarchived_at.iter().map(|(id, at)| (id.to_string(), *at)).collect(),
        }
    }

    /// A transcript directory with one file per `(id, age)`, each last written `age`
    /// before [`NOW`].
    fn transcripts(files: &[(&str, u64)]) -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        for (id, age_ms) in files {
            let path = tmp.path().join(format!("{id}.jsonl"));
            fs::write(&path, "{}\n").unwrap();
            let when = UNIX_EPOCH + Duration::from_millis(NOW - age_ms);
            fs::File::options().write(true).open(&path).unwrap().set_modified(when).unwrap();
        }
        tmp
    }

    fn rows(of: &[&str]) -> String {
        let rows: Vec<serde_json::Value> = of
            .iter()
            .map(|id| serde_json::json!({ "sessionId": id, "display": format!("Title of {id}"), "timestamp": "2027-01-01T00:00:00Z" }))
            .collect();
        serde_json::Value::Array(rows).to_string()
    }

    /// `apply_in`'s answer as `(ids marked archived, ids archived by this call)`.
    fn applied(json: &str) -> (Vec<String>, Vec<String>) {
        let out: serde_json::Value = serde_json::from_str(json).expect("apply answers in JSON");
        let marked = out["sessions"]
            .as_array()
            .expect("a sessions array")
            .iter()
            .filter(|r| r["archived"] == true)
            .map(|r| r["sessionId"].as_str().unwrap().to_string())
            .collect();
        let now = out["archivedNow"].as_array().expect("an archivedNow array").iter().map(|v| v.as_str().unwrap().to_string()).collect();
        (marked, now)
    }

    #[test]
    fn only_the_periods_the_preference_offers_count_and_anything_else_is_never() {
        assert_eq!([period(1), period(2), period(7), period(14)], [1, 2, 7, 14]);
        assert_eq!([period(0), period(3), period(30), period(-1), period(i64::MAX)], [0; 5]);
    }

    #[test]
    fn set_to_never_nothing_is_archived_however_old() {
        let sessions = [listed("ancient", 1)];

        assert_eq!(inactive(&sessions, 0, NOW, &set_of(&[]), &store(&[], &[])), ids(&[]));
    }

    #[test]
    fn a_conversation_idle_for_the_whole_period_is_archived_and_a_newer_one_is_not() {
        let sessions = [
            listed("month-old", NOW - 30 * DAY_MS),
            listed("a-week-to-the-ms", NOW - WEEK * DAY_MS),
            listed("a-ms-short-of-a-week", NOW - WEEK * DAY_MS + 1),
            listed("today", NOW),
        ];

        assert_eq!(
            inactive(&sessions, WEEK, NOW, &set_of(&[]), &store(&[], &[])),
            ids(&["month-old", "a-week-to-the-ms"])
        );
    }

    #[test]
    fn a_conversation_in_use_is_never_archived() {
        let sessions = [listed("open-in-a-tab", NOW - 30 * DAY_MS), listed("closed", NOW - 30 * DAY_MS)];

        assert_eq!(
            inactive(&sessions, WEEK, NOW, &set_of(&["open-in-a-tab"]), &store(&[], &[])),
            ids(&["closed"])
        );
    }

    #[test]
    fn one_already_archived_is_not_archived_again() {
        let sessions = [listed("already", NOW - 30 * DAY_MS), listed("new", NOW - 30 * DAY_MS)];

        assert_eq!(inactive(&sessions, WEEK, NOW, &set_of(&[]), &store(&["already"], &[])), ids(&["new"]));
    }

    #[test]
    fn one_whose_file_time_is_not_known_is_left_alone() {
        let sessions = [listed("no-file", 0)];

        assert_eq!(inactive(&sessions, WEEK, NOW, &set_of(&[]), &store(&[], &[])), ids(&[]));
    }

    #[test]
    fn one_taken_out_of_the_archive_is_left_alone_for_the_period_and_archived_after_it() {
        let sessions = [listed("yesterday", NOW - 30 * DAY_MS), listed("last-month", NOW - 30 * DAY_MS)];
        let taken_out = store(&[], &[("yesterday", NOW - DAY_MS), ("last-month", NOW - 29 * DAY_MS)]);

        assert_eq!(inactive(&sessions, WEEK, NOW, &set_of(&[]), &taken_out), ids(&["last-month"]));
    }

    #[test]
    fn a_clock_earlier_than_the_period_archives_nothing() {
        let sessions = [listed("any", 1)];

        assert_eq!(inactive(&sessions, WEEK, DAY_MS, &set_of(&[]), &store(&[], &[])), ids(&[]));
    }

    #[test]
    fn a_missing_or_unreadable_store_is_an_empty_archive() {
        let tmp = tempfile::tempdir().unwrap();
        let garbled = tmp.path().join("garbled.json");
        fs::write(&garbled, "{ not json").unwrap();
        let other_shape = tmp.path().join("array.json");
        fs::write(&other_shape, "[1, 2]").unwrap();

        for path in [tmp.path().join("missing.json"), garbled, other_shape] {
            let store = read_store(&path);
            assert!(store.archived.is_empty() && store.unarchived_at.is_empty(), "{}", path.display());
        }
    }

    #[test]
    fn archiving_is_recorded_once_per_conversation_and_read_back() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("state").join("archive.json");
        let at = path.to_str().unwrap();

        assert!(set_at(at, &ids(&["a", "b"]), true, NOW));
        assert!(set_at(at, &ids(&["b", "c"]), true, NOW));

        assert_eq!(read_store(&path).archived, ids(&["a", "b", "c"]));
        assert!(!tmp.path().join("state").join("archive.json.tmp").exists(), "the temporary file is gone");
    }

    #[test]
    fn unarchiving_takes_the_conversation_out_and_remembers_when() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("archive.json");
        let at = path.to_str().unwrap();
        assert!(set_at(at, &ids(&["a", "b"]), true, NOW - DAY_MS));

        assert!(set_at(at, &ids(&["a"]), false, NOW));

        let store = read_store(&path);
        assert_eq!(store.archived, ids(&["b"]));
        assert_eq!(store.unarchived_at.get("a"), Some(&NOW));
        assert_eq!(store.unarchived_at.len(), 1);
    }

    #[test]
    fn an_unarchive_older_than_the_longest_period_is_forgotten() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("archive.json");
        assert!(write_store(&path, &store(&["x"], &[("stale", NOW - 14 * DAY_MS), ("recent", NOW - 14 * DAY_MS + 1)])));

        assert!(set_at(path.to_str().unwrap(), &ids(&["x"]), false, NOW));

        let kept: Vec<String> = read_store(&path).unarchived_at.into_keys().collect();
        assert_eq!(kept, ids(&["recent", "x"]));
    }

    #[test]
    fn without_a_store_to_write_to_nothing_is_recorded() {
        assert!(!set_at("", &ids(&["a"]), true, NOW));
    }

    #[test]
    fn a_transcripts_file_time_is_read_in_ms_and_an_id_that_is_no_file_name_has_none() {
        let dir = transcripts(&[("a", 3 * DAY_MS)]);
        fs::write(dir.path().join("outside.jsonl"), "{}\n").unwrap();
        let inner = dir.path().join("inner");
        fs::create_dir(&inner).unwrap();

        assert_eq!(modified_ms(dir.path(), "a"), NOW - 3 * DAY_MS);
        assert_eq!(modified_ms(dir.path(), "missing"), 0);
        assert_eq!(modified_ms(dir.path(), ""), 0);
        assert_eq!(modified_ms(&inner, "../outside"), 0);
        assert_eq!(modified_ms(&inner, "..\\outside"), 0);
    }

    #[test]
    fn the_list_comes_back_marked_with_the_inactive_conversations_archived() {
        let dir = transcripts(&[("old", 30 * DAY_MS), ("new", DAY_MS)]);
        let store_path = dir.path().join("archive.json");

        let json = apply_in(&store_path, Some(dir.path()), &rows(&["new", "old"]), WEEK, &set_of(&[]), NOW);

        assert_eq!(applied(&json), (ids(&["old"]), ids(&["old"])));
        assert_eq!(read_store(&store_path).archived, ids(&["old"]));
        // The rows are the caller's own, with one field added.
        let out: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(out["sessions"][0], serde_json::json!({
            "sessionId": "new", "display": "Title of new", "timestamp": "2027-01-01T00:00:00Z", "archived": false
        }));
    }

    #[test]
    fn a_second_load_archives_nothing_new_and_still_marks_what_is_archived() {
        let dir = transcripts(&[("old", 30 * DAY_MS), ("new", DAY_MS)]);
        let store_path = dir.path().join("archive.json");
        apply_in(&store_path, Some(dir.path()), &rows(&["new", "old"]), WEEK, &set_of(&[]), NOW);

        let json = apply_in(&store_path, Some(dir.path()), &rows(&["new", "old"]), WEEK, &set_of(&[]), NOW);

        assert_eq!(applied(&json), (ids(&["old"]), ids(&[])));
    }

    #[test]
    fn a_conversation_open_in_a_tab_is_passed_over_by_the_sweep() {
        let dir = transcripts(&[("old", 30 * DAY_MS), ("old-but-open", 30 * DAY_MS)]);
        let store_path = dir.path().join("archive.json");

        let json = apply_in(&store_path, Some(dir.path()), &rows(&["old", "old-but-open"]), WEEK, &set_of(&["old-but-open"]), NOW);

        assert_eq!(applied(&json), (ids(&["old"]), ids(&["old"])));
    }

    #[test]
    fn set_to_never_the_sweep_archives_nothing_and_what_was_archived_still_shows() {
        let dir = transcripts(&[("old", 30 * DAY_MS), ("older", 60 * DAY_MS)]);
        let store_path = dir.path().join("archive.json");
        assert!(set_at(store_path.to_str().unwrap(), &ids(&["older"]), true, NOW));

        let json = apply_in(&store_path, Some(dir.path()), &rows(&["old", "older"]), 0, &set_of(&[]), NOW);

        assert_eq!(applied(&json), (ids(&["older"]), ids(&[])));
    }

    #[test]
    fn one_unarchived_by_hand_stays_out_on_the_next_load() {
        let dir = transcripts(&[("old", 30 * DAY_MS)]);
        let store_path = dir.path().join("archive.json");
        apply_in(&store_path, Some(dir.path()), &rows(&["old"]), WEEK, &set_of(&[]), NOW);
        assert!(set_at(store_path.to_str().unwrap(), &ids(&["old"]), false, NOW));

        let json = apply_in(&store_path, Some(dir.path()), &rows(&["old"]), WEEK, &set_of(&[]), NOW + DAY_MS);

        assert_eq!(applied(&json), (ids(&[]), ids(&[])));
    }

    #[test]
    fn an_archive_that_cannot_be_recorded_is_not_shown_as_one() {
        let dir = transcripts(&[("old", 30 * DAY_MS)]);
        // A directory with something in it where the store should be: it cannot be replaced.
        let store_path = dir.path().join("archive.json");
        fs::create_dir(&store_path).unwrap();
        fs::write(store_path.join("in-the-way"), "").unwrap();

        let json = apply_in(&store_path, Some(dir.path()), &rows(&["old"]), WEEK, &set_of(&[]), NOW);

        assert_eq!(applied(&json), (ids(&[]), ids(&[])));
        assert!(!dir.path().join("archive.json.tmp").exists(), "no temporary file is left behind");
    }

    #[test]
    fn without_a_transcript_directory_nothing_is_archived_and_the_rows_are_still_marked() {
        let tmp = tempfile::tempdir().unwrap();
        let store_path = tmp.path().join("archive.json");
        assert!(set_at(store_path.to_str().unwrap(), &ids(&["a"]), true, NOW));

        let json = apply_in(&store_path, None, &rows(&["a", "b"]), WEEK, &set_of(&[]), NOW);

        assert_eq!(applied(&json), (ids(&["a"]), ids(&[])));
    }

    #[test]
    fn a_list_that_is_not_one_comes_back_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let store_path = tmp.path().join("archive.json");

        for not_a_list in ["", "not json", "{\"sessionId\":\"a\"}"] {
            let json = apply_in(&store_path, None, not_a_list, WEEK, &set_of(&[]), NOW);
            let out: serde_json::Value = serde_json::from_str(&json).expect("still JSON");
            assert_eq!(out, serde_json::json!({ "sessions": [], "archivedNow": [] }), "{not_a_list:?}");
        }
    }

    #[test]
    fn the_callers_ids_arrive_as_json_and_a_list_that_is_not_one_is_no_ids() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("archive.json");
        let at = path.to_str().unwrap();

        assert!(set(at, "[\"a\",\"b\"]", true));
        assert!(set(at, "not json", true));
        assert!(set(at, "[\"a\"]", false));

        let store = read_store(&path);
        assert_eq!(store.archived, ids(&["b"]));
        assert!(store.unarchived_at.contains_key("a"));
    }
}
