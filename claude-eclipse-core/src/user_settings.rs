//! The user's own settings file (`~/.claude/settings.json`), for the one setting the
//! command menu saves there.
//!
//! The CLI will not save this one itself: asked to with `update_settings` on the user
//! layer it answers "keys not allowed: switchModelsOnFlag" (run on 2.1.291). The VS Code
//! panel edits the file itself and then tells the running session, and so does this,
//! with one difference: only that setting's text is changed, so the rest of the file
//! stays byte for byte as its owner wrote it.

use std::ops::Range;
use std::path::{Path, PathBuf};

/// The settings the page may save here. Each is a switch.
const WRITABLE: [&str; 1] = ["switchModelsOnFlag"];

/// Whether `key` is one of the settings saved here and `value` is what it takes.
fn allowed(key: &str, value: &serde_json::Value) -> bool {
    WRITABLE.contains(&key) && value.is_boolean()
}

/// Where the user's settings file is: in the folder `CLAUDE_CONFIG_DIR` names when it
/// names one, which is where the CLI then reads it, and in `~/.claude` otherwise.
fn settings_path() -> Option<PathBuf> {
    let folder = match std::env::var_os("CLAUDE_CONFIG_DIR") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => crate::session::dirs_home()?.join(".claude"),
    };
    Some(folder.join("settings.json"))
}

/// One member of the file's top-level object: its key, and where its value's text is.
struct Member {
    key: String,
    key_start: usize,
    value: Range<usize>,
}

/// The index just past the string that opens at `at`.
fn string_end(bytes: &[u8], at: usize) -> usize {
    let mut i = at + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'"' => return i + 1,
            _ => i += 1,
        }
    }
    bytes.len()
}

/// The index just past the value that starts at `at`: a string, an object or a list
/// with everything in it, or a number or word.
fn value_end(bytes: &[u8], at: usize) -> usize {
    match bytes[at] {
        b'"' => string_end(bytes, at),
        b'{' | b'[' => {
            let (mut i, mut depth) = (at, 0usize);
            while i < bytes.len() {
                match bytes[i] {
                    b'"' => {
                        i = string_end(bytes, i);
                        continue;
                    }
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' => {
                        depth -= 1;
                        if depth == 0 {
                            return i + 1;
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
            bytes.len()
        }
        _ => {
            let mut i = at;
            while i < bytes.len() && !matches!(bytes[i], b',' | b'}' | b']') && !bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            i
        }
    }
}

/// Where the top-level object of `text` opens and closes, and its members in the order
/// they are written. `text` is JSON that has already been read as an object.
fn top_level(text: &str) -> Option<(usize, Vec<Member>, usize)> {
    let bytes = text.as_bytes();
    let skip = |mut i: usize| {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        i
    };
    let open = skip(0);
    if bytes.get(open) != Some(&b'{') {
        return None;
    }
    let mut members = Vec::new();
    let mut i = skip(open + 1);
    loop {
        match *bytes.get(i)? {
            b'}' => return Some((open, members, i)),
            b',' => i = skip(i + 1),
            b'"' => {
                let key_end = string_end(bytes, i);
                let key: String = serde_json::from_str(text.get(i..key_end)?).ok()?;
                let colon = skip(key_end);
                if bytes.get(colon) != Some(&b':') {
                    return None;
                }
                let start = skip(colon + 1);
                if start >= bytes.len() {
                    return None;
                }
                let end = value_end(bytes, start);
                members.push(Member { key, key_start: i, value: start..end });
                i = skip(end);
            }
            _ => return None,
        }
    }
}

/// `text`, the content of a settings file, with `key` set to `value` and nothing else
/// changed: the setting's own value is rewritten where it stands, or the setting is
/// added after the last one, written as the others are. An empty file becomes one
/// holding just the setting. An `Err` says why the file cannot be changed.
pub(crate) fn with_key(text: &str, key: &str, value: &serde_json::Value) -> Result<String, String> {
    let (mark, body) = match text.strip_prefix('\u{feff}') {
        Some(rest) => ("\u{feff}", rest),
        None => ("", text),
    };
    let line_end = if body.contains("\r\n") { "\r\n" } else { "\n" };
    let member = format!("{}: {value}", serde_json::Value::from(key));
    if body.trim().is_empty() {
        return Ok(format!("{mark}{{{line_end}  {member}{line_end}}}{line_end}"));
    }
    let before: serde_json::Value = serde_json::from_str(body).map_err(|_| "Settings file is not valid JSON".to_string())?;
    if !before.is_object() {
        return Err("Settings file is not a JSON object".into());
    }
    let unsafe_change = || "The settings file could not be changed safely.".to_string();
    let (open, members, close) = top_level(body).ok_or_else(unsafe_change)?;

    // A key written twice counts as its last writing, to this as to anything reading it.
    let changed = if let Some(found) = members.iter().rev().find(|m| m.key == key) {
        format!("{}{value}{}", &body[..found.value.start], &body[found.value.end..])
    } else if let (Some(first), Some(last)) = (members.first(), members.last()) {
        // Written as the first setting is: on a line of its own with that indent, or
        // on the one line the whole file is on.
        let line_start = body[..first.key_start].rfind('\n').map(|n| n + 1);
        let lead = match line_start {
            Some(start) if body[start..first.key_start].chars().all(|c| c == ' ' || c == '\t') => {
                format!(",{line_end}{}", &body[start..first.key_start])
            }
            _ => ", ".to_string(),
        };
        format!("{}{lead}{member}{}", &body[..last.value.end], &body[last.value.end..])
    } else {
        format!("{}{line_end}  {member}{line_end}{}", &body[..=open], &body[close..])
    };

    // The check that makes the text edit safe: read back, the file must be exactly what
    // it was with this one setting set.
    let mut wanted = before;
    wanted[key] = value.clone();
    if serde_json::from_str::<serde_json::Value>(&changed).ok().as_ref() != Some(&wanted) {
        return Err(unsafe_change());
    }
    Ok(format!("{mark}{changed}"))
}

/// Sets `key` in the settings file at `path`, making the file and its folder when they
/// are not there. A file that is a link is written through to what it points at, and
/// the file keeps its permissions.
fn set_in(path: &Path, key: &str, value: &serde_json::Value) -> Result<(), String> {
    let cannot_save = |e: std::io::Error| format!("The settings were not saved: {e}");
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("The settings file could not be read: {e}")),
    };
    let changed = with_key(&text, key, value).map_err(|why| format!("{why}: {}", path.display()))?;
    if changed == text {
        return Ok(());
    }
    // Replaced whole, by a file written beside it, so nothing reading it sees half.
    let target = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let folder = target.parent().ok_or_else(|| "The settings were not saved: no folder.".to_string())?;
    std::fs::create_dir_all(folder).map_err(cannot_save)?;
    let name = target.file_name().unwrap_or_default().to_string_lossy();
    let tmp = folder.join(format!("{name}.tmp"));
    let written = std::fs::write(&tmp, &changed).and_then(|()| {
        if let Ok(meta) = std::fs::metadata(&target) {
            std::fs::set_permissions(&tmp, meta.permissions())?;
        }
        std::fs::rename(&tmp, &target)
    });
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written.map_err(cannot_save)
}

/// What [`set`] returns: `{"ok":true}`, or `{"ok":false,"error"}` with why not.
fn set_json(result: Result<(), String>) -> String {
    match result {
        Ok(()) => serde_json::json!({ "ok": true }),
        Err(error) => serde_json::json!({ "ok": false, "error": error }),
    }
    .to_string()
}

/// Saves one of the command menu's settings in the user's settings file. `value_json`
/// is the value as JSON.
pub fn set(key: &str, value_json: &str) -> String {
    let result = match serde_json::from_str::<serde_json::Value>(value_json) {
        Ok(value) if allowed(key, &value) => match settings_path() {
            Some(path) => set_in(&path, key, &value),
            None => Err("The settings were not saved: no home folder.".into()),
        },
        _ => Err("Invalid request.".into()),
    };
    if crate::is_debug() {
        match &result {
            Ok(()) => eprintln!("[user-settings] saved {key} = {value_json}"),
            Err(error) => eprintln!("[user-settings] {key} not saved: {error}"),
        }
    }
    set_json(result)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::EnvGuard;

    const KEY: &str = "switchModelsOnFlag";

    fn off(text: &str) -> Result<String, String> {
        with_key(text, KEY, &serde_json::Value::Bool(false))
    }

    #[test]
    fn an_empty_file_becomes_one_holding_just_the_setting() {
        assert_eq!(off("").unwrap(), "{\n  \"switchModelsOnFlag\": false\n}\n");
        assert_eq!(off("  \n").unwrap(), "{\n  \"switchModelsOnFlag\": false\n}\n");
    }

    #[test]
    fn the_rest_of_the_file_stays_byte_for_byte() {
        let text = "{\n  \"zeta\": 1,\n  \"alpha\": { \"b\": 2, \"a\": 1 },\n  \"model\": \"sonnet\"\n}\n";
        assert_eq!(
            off(text).unwrap(),
            "{\n  \"zeta\": 1,\n  \"alpha\": { \"b\": 2, \"a\": 1 },\n  \"model\": \"sonnet\",\n  \"switchModelsOnFlag\": false\n}\n"
        );
    }

    #[test]
    fn a_setting_already_there_has_only_its_value_changed() {
        let text = "{\n  \"a\": 1,\n  \"switchModelsOnFlag\":   true,\n  \"b\": [1, 2]\n}";
        assert_eq!(off(text).unwrap(), "{\n  \"a\": 1,\n  \"switchModelsOnFlag\":   false,\n  \"b\": [1, 2]\n}");
        let on = with_key(&off(text).unwrap(), KEY, &serde_json::Value::Bool(true)).unwrap();
        assert_eq!(on, text, "and switching it back gives the file it was");
    }

    #[test]
    fn the_same_words_deeper_in_or_inside_a_string_are_left_alone() {
        let text = "{\n\t\"x\": {\"switchModelsOnFlag\": true},\n\t\"note\": \"\\\"switchModelsOnFlag\\\": true, }\"\n}";
        assert_eq!(
            off(text).unwrap(),
            "{\n\t\"x\": {\"switchModelsOnFlag\": true},\n\t\"note\": \"\\\"switchModelsOnFlag\\\": true, }\",\n\t\"switchModelsOnFlag\": false\n}"
        );
    }

    #[test]
    fn a_file_on_one_line_stays_on_one_line() {
        assert_eq!(off("{\"a\":1}").unwrap(), "{\"a\":1, \"switchModelsOnFlag\": false}");
    }

    #[test]
    fn an_empty_object_gets_the_setting() {
        assert_eq!(off("{}").unwrap(), "{\n  \"switchModelsOnFlag\": false\n}");
        assert_eq!(off("{ }\n").unwrap(), "{\n  \"switchModelsOnFlag\": false\n}\n");
    }

    #[test]
    fn windows_line_ends_and_tabs_are_kept() {
        assert_eq!(off("{\r\n\t\"a\": 1\r\n}\r\n").unwrap(), "{\r\n\t\"a\": 1,\r\n\t\"switchModelsOnFlag\": false\r\n}\r\n");
    }

    #[test]
    fn a_byte_order_mark_is_kept() {
        assert_eq!(off("\u{feff}{\"a\":1}").unwrap(), "\u{feff}{\"a\":1, \"switchModelsOnFlag\": false}");
    }

    #[test]
    fn a_key_written_twice_is_changed_where_it_counts() {
        let text = "{\"switchModelsOnFlag\": true, \"switchModelsOnFlag\": true}";
        assert_eq!(off(text).unwrap(), "{\"switchModelsOnFlag\": true, \"switchModelsOnFlag\": false}");
    }

    #[test]
    fn a_file_that_is_not_a_json_object_is_refused() {
        assert_eq!(off("{oops").unwrap_err(), "Settings file is not valid JSON");
        assert_eq!(off("// mine\n{}").unwrap_err(), "Settings file is not valid JSON");
        assert_eq!(off("[1]").unwrap_err(), "Settings file is not a JSON object");
    }

    #[test]
    fn only_the_settings_the_menu_saves_are_saved() {
        assert!(allowed(KEY, &serde_json::json!(true)));
        assert!(allowed(KEY, &serde_json::json!(false)));
        assert!(!allowed(KEY, &serde_json::json!("false")));
        assert!(!allowed(KEY, &serde_json::Value::Null));
        assert!(!allowed("permissions", &serde_json::json!(true)));
        assert!(!allowed("env", &serde_json::json!({})));
    }

    #[test]
    fn the_file_is_in_the_config_folder_the_cli_reads() {
        let mut env = EnvGuard::lock();
        let home = tempfile::tempdir().unwrap();
        env.set_home(home.path());
        env.remove("CLAUDE_CONFIG_DIR");
        assert_eq!(settings_path().unwrap(), home.path().join(".claude").join("settings.json"));
        env.set("CLAUDE_CONFIG_DIR", ""); // empty counts as unset
        assert_eq!(settings_path().unwrap(), home.path().join(".claude").join("settings.json"));
        let moved = tempfile::tempdir().unwrap();
        env.set("CLAUDE_CONFIG_DIR", moved.path());
        assert_eq!(settings_path().unwrap(), moved.path().join("settings.json"));
    }

    #[test]
    fn saving_makes_the_file_then_changes_it_and_leaves_nothing_else_behind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".claude").join("settings.json");
        set_in(&path, KEY, &serde_json::json!(false)).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\n  \"switchModelsOnFlag\": false\n}\n");
        set_in(&path, KEY, &serde_json::json!(true)).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\n  \"switchModelsOnFlag\": true\n}\n");
        let names: Vec<_> = std::fs::read_dir(path.parent().unwrap()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, ["settings.json"], "no temporary file is left behind");
    }

    #[test]
    fn a_file_that_cannot_be_read_as_settings_is_left_as_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{ \"model\": \"sonnet\", }").unwrap();
        let error = set_in(&path, KEY, &serde_json::json!(false)).unwrap_err();
        assert!(error.starts_with("Settings file is not valid JSON: "), "{error}");
        assert!(error.ends_with("settings.json"), "{error}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ \"model\": \"sonnet\", }");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn the_whole_call_saves_under_the_config_folder_and_says_so() {
        let mut env = EnvGuard::lock();
        let config = tempfile::tempdir().unwrap();
        env.set("CLAUDE_CONFIG_DIR", config.path());
        std::fs::write(config.path().join("settings.json"), "{\n  \"model\": \"sonnet\"\n}\n").unwrap();
        assert_eq!(set(KEY, "false"), r#"{"ok":true}"#);
        assert_eq!(
            std::fs::read_to_string(config.path().join("settings.json")).unwrap(),
            "{\n  \"model\": \"sonnet\",\n  \"switchModelsOnFlag\": false\n}\n"
        );
    }

    #[test]
    fn a_request_for_another_setting_or_another_kind_of_value_writes_nothing() {
        let mut env = EnvGuard::lock();
        let config = tempfile::tempdir().unwrap();
        env.set("CLAUDE_CONFIG_DIR", config.path());
        for (key, value) in [("permissions", "true"), (KEY, "\"yes\""), (KEY, "not json"), ("", "true")] {
            assert_eq!(set(key, value), r#"{"error":"Invalid request.","ok":false}"#, "{key} = {value}");
        }
        assert_eq!(std::fs::read_dir(config.path()).unwrap().count(), 0);
    }

    #[test]
    fn a_failure_is_reported_with_its_reason() {
        assert_eq!(set_json(Err("no".into())), r#"{"error":"no","ok":false}"#);
        assert_eq!(set_json(Ok(())), r#"{"ok":true}"#);
    }

    // A settings file kept elsewhere and linked into place stays a link.
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "freebsd"))]
    #[test]
    fn a_linked_settings_file_is_written_through_the_link() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("dotfiles").join("claude-settings.json");
        std::fs::create_dir_all(real.parent().unwrap()).unwrap();
        std::fs::write(&real, "{}\n").unwrap();
        let link = dir.path().join("settings.json");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        set_in(&link, KEY, &serde_json::json!(false)).unwrap();
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink(), "still a link");
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "{\n  \"switchModelsOnFlag\": false\n}\n");
    }
}
