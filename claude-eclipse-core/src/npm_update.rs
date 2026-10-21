//! Clears what an earlier Claude Code update left in npm's global folder, so the
//! next `claude update` can run. Windows only.
//!
//! npm updates a package by first moving the old copy aside to a folder whose name
//! it derives from the package path alone (`.claude-code-<8 chars of a sha1>`, from
//! `@npmcli/arborist/lib/retire-path.js`), so every update uses the same name. It
//! deletes that folder at the end, but on Windows it cannot: the package's
//! `install.cjs` hard-links `bin\claude.exe` to the native binary, and every chat
//! tab still running the old version holds that file open under both names.
//!
//! The next update then finds the folder already there. A directory cannot be
//! renamed onto one that exists, so npm falls back to copying file by file
//! (`@npmcli/fs/lib/move-file.js`), hits the binary still locked inside the old
//! folder (`EBUSY ... copyfile`), rolls back and fails. Its rollback deletes the
//! folder, which is why a second attempt always worked.
//!
//! [`clear_leftovers_json`] does that clearing before the first attempt. A folder
//! that is still locked is renamed instead, which Windows allows while a program
//! inside it runs; the new name also starts with a dot, which npm skips when it
//! reads `node_modules` ("ignore . dirs and retired scoped package folders",
//! `load-actual.js`). Those are deleted on a later update, once nothing holds them.
//!
//! macOS, Linux and FreeBSD delete a file that is still open, so npm's own cleanup
//! succeeds there and nothing is ever left behind. Each has its own entry point
//! that does nothing.

/// The report for an install this module has nothing to do for.
#[cfg(target_os = "macos")]
const NOT_AN_NPM_INSTALL: &str = r#"{"npm":false}"#;
#[cfg(target_os = "linux")]
const NOT_AN_NPM_INSTALL: &str = r#"{"npm":false}"#;
#[cfg(target_os = "freebsd")]
const NOT_AN_NPM_INSTALL: &str = r#"{"npm":false}"#;

// ───────────────────────────────────────────────────────────────────────────
// macOS
// ───────────────────────────────────────────────────────────────────────────

/// Nothing to clear: npm's own cleanup succeeds on macOS.
#[cfg(target_os = "macos")]
pub fn clear_leftovers_json(_claude_cmd: &str) -> String {
    NOT_AN_NPM_INSTALL.to_string()
}

// ───────────────────────────────────────────────────────────────────────────
// Linux
// ───────────────────────────────────────────────────────────────────────────

/// Nothing to clear: npm's own cleanup succeeds on Linux.
#[cfg(target_os = "linux")]
pub fn clear_leftovers_json(_claude_cmd: &str) -> String {
    NOT_AN_NPM_INSTALL.to_string()
}

// ───────────────────────────────────────────────────────────────────────────
// FreeBSD
// ───────────────────────────────────────────────────────────────────────────

/// Nothing to clear: npm's own cleanup succeeds on FreeBSD.
#[cfg(target_os = "freebsd")]
pub fn clear_leftovers_json(_claude_cmd: &str) -> String {
    NOT_AN_NPM_INSTALL.to_string()
}

// ───────────────────────────────────────────────────────────────────────────
// Windows
// ───────────────────────────────────────────────────────────────────────────

#[cfg(windows)]
use std::path::{Path, PathBuf};

/// Every folder npm moves `@anthropic-ai/claude-code` aside to starts with this.
#[cfg(windows)]
const RETIRED_PREFIX: &str = ".claude-code-";

/// The name a still-locked leftover is renamed to, followed by a timestamp.
#[cfg(windows)]
const STALE_PREFIX: &str = ".claude-code-stale-";

/// What [`clear_leftovers`] found and did, by folder name.
#[cfg(windows)]
#[derive(Default, Debug, PartialEq)]
struct Leftovers {
    /// The command belongs to an npm install of Claude Code.
    npm: bool,
    /// Deleted.
    removed: Vec<String>,
    /// Still locked, so renamed; each entry is the NEW name.
    set_aside: Vec<String>,
    /// Neither deleted nor renamed.
    kept: Vec<String>,
}

#[cfg(windows)]
impl Leftovers {
    fn to_json(&self) -> String {
        serde_json::json!({
            "npm": self.npm,
            "removed": self.removed,
            "setAside": self.set_aside,
            "kept": self.kept,
        })
        .to_string()
    }
}

/// Clears the leftovers of the npm install `claude_cmd` belongs to (the resolved
/// `claude.cmd`, or the `claude.exe` inside the package) and reports what it did as
/// JSON: `{"npm":bool,"removed":[..],"setAside":[..],"kept":[..]}`.
#[cfg(windows)]
pub fn clear_leftovers_json(claude_cmd: &str) -> String {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    clear_leftovers(Path::new(claude_cmd), stamp).to_json()
}

/// [`clear_leftovers_json`] with the timestamp for renamed folders passed in.
#[cfg(windows)]
fn clear_leftovers(claude_cmd: &Path, stamp: u128) -> Leftovers {
    match npm_scope(claude_cmd) {
        Some(scope) => clear_retired(&scope, stamp),
        None => Leftovers::default(),
    }
}

/// The `node_modules\@anthropic-ai` folder of the npm install `claude_cmd` belongs
/// to, or None when it is not one. Only two layouts count: npm's `claude.cmd` shim
/// in the prefix itself, or a path inside the package. A folder merely somewhere
/// above (a local install in the home folder, under a native `~\.local\bin`) is not
/// the install that command runs.
#[cfg(windows)]
fn npm_scope(claude_cmd: &Path) -> Option<PathBuf> {
    let is_package = |scope: &Path| scope.join("claude-code").join("package.json").is_file();
    let beside_shim = claude_cmd.parent()?.join("node_modules").join("@anthropic-ai");
    if is_package(&beside_shim) {
        return Some(beside_shim);
    }
    claude_cmd
        .ancestors()
        .find(|dir| {
            dir.file_name().is_some_and(|n| n.eq_ignore_ascii_case("claude-code"))
                && dir.parent().and_then(Path::file_name).is_some_and(|n| n.eq_ignore_ascii_case("@anthropic-ai"))
                && dir.parent().and_then(Path::parent).and_then(Path::file_name)
                    .is_some_and(|n| n.eq_ignore_ascii_case("node_modules"))
        })
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .filter(|scope| is_package(scope))
}

/// Deletes, or failing that renames, every `.claude-code-*` folder in `scope`. A
/// folder already renamed by an earlier run is only ever deleted: it no longer has
/// npm's name, so it is not in the way.
#[cfg(windows)]
fn clear_retired(scope: &Path, stamp: u128) -> Leftovers {
    let mut out = Leftovers { npm: true, ..Leftovers::default() };
    let Ok(entries) = std::fs::read_dir(scope) else { return out };
    // file_type() does not follow links: a junction or symlink is never walked into.
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(RETIRED_PREFIX))
        .collect();
    names.sort();
    for name in names {
        let path = scope.join(&name);
        if std::fs::remove_dir_all(&path).is_ok() {
            out.removed.push(name);
        } else if name.starts_with(STALE_PREFIX) {
            out.kept.push(name);
        } else {
            match set_aside(scope, &path, stamp) {
                Some(new_name) => out.set_aside.push(new_name),
                None => out.kept.push(name),
            }
        }
    }
    out
}

/// Renames `path` to the first free `.claude-code-stale-<stamp>[-n]` in `scope`.
#[cfg(windows)]
fn set_aside(scope: &Path, path: &Path, stamp: u128) -> Option<String> {
    (0..100)
        .map(|n| match n {
            0 => format!("{STALE_PREFIX}{stamp}"),
            n => format!("{STALE_PREFIX}{stamp}-{n}"),
        })
        .find(|name| !scope.join(name).exists())
        .filter(|name| std::fs::rename(path, scope.join(name)).is_ok())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(all(test, target_os = "macos"))]
mod macos_tests {
    use super::*;

    #[test]
    fn there_is_never_anything_to_clear() {
        assert_eq!(clear_leftovers_json("/usr/local/bin/claude"), r#"{"npm":false}"#);
    }
}

#[cfg(all(test, target_os = "linux"))]
mod linux_tests {
    use super::*;

    #[test]
    fn there_is_never_anything_to_clear() {
        assert_eq!(clear_leftovers_json("/usr/local/bin/claude"), r#"{"npm":false}"#);
    }
}

#[cfg(all(test, target_os = "freebsd"))]
mod freebsd_tests {
    use super::*;

    #[test]
    fn there_is_never_anything_to_clear() {
        assert_eq!(clear_leftovers_json("/usr/local/bin/claude"), r#"{"npm":false}"#);
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;
    use std::fs;
    use std::process::{Child, Command, Stdio};

    /// An npm prefix holding an install of Claude Code, as `npm i -g` lays it out:
    /// `claude.cmd` at the top, the package under `node_modules\@anthropic-ai`.
    fn npm_prefix() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let prefix = dir.path().join("npm");
        let scope = prefix.join("node_modules").join("@anthropic-ai");
        fs::create_dir_all(scope.join("claude-code").join("bin")).unwrap();
        fs::write(scope.join("claude-code").join("package.json"), "{}").unwrap();
        fs::write(scope.join("claude-code").join("bin").join("claude.exe"), "new").unwrap();
        fs::write(prefix.join("claude.cmd"), "@echo off").unwrap();
        (dir, prefix, scope)
    }

    /// A folder npm moved the old package into, holding `file` (relative) as `contents`.
    fn retired(scope: &Path, name: &str, file: &str, contents: &[u8]) -> PathBuf {
        let path = scope.join(name).join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, contents).unwrap();
        path
    }

    fn names(scope: &Path) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(scope)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    /// A program running from a copied exe, killed when dropped (before the temp
    /// folder holding it is deleted). This is how a chat tab holds the old binary.
    struct Running(Child);

    impl Running {
        fn from(exe: &Path) -> Running {
            let system = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
            fs::copy(Path::new(&system).join("System32").join("PING.EXE"), exe).unwrap();
            let child = Command::new(exe)
                .args(["-n", "120", "127.0.0.1"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            Running(child)
        }
    }

    impl Drop for Running {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[test]
    fn a_leftover_nothing_holds_is_deleted() {
        let (_dir, prefix, scope) = npm_prefix();
        retired(&scope, ".claude-code-QrJhBTEm", r"bin\claude.exe.old.1792593565991", b"old");

        let report = clear_leftovers(&prefix.join("claude.cmd"), 7);

        assert_eq!(
            report,
            Leftovers { npm: true, removed: vec![".claude-code-QrJhBTEm".into()], ..Leftovers::default() }
        );
        assert_eq!(names(&scope), vec!["claude-code"]);
        assert_eq!(fs::read(scope.join(r"claude-code\bin\claude.exe")).unwrap(), b"new");
    }

    #[test]
    fn a_leftover_a_running_program_holds_is_renamed_out_of_npms_way() {
        let (_dir, prefix, scope) = npm_prefix();
        let exe = scope
            .join(".claude-code-QrJhBTEm")
            .join(r"node_modules\@anthropic-ai\claude-code-win32-x64\claude.exe");
        fs::create_dir_all(exe.parent().unwrap()).unwrap();
        let running = Running::from(&exe);

        let report = clear_leftovers(&prefix.join("claude.cmd"), 1792593565991);

        assert_eq!(
            report,
            Leftovers {
                npm: true,
                set_aside: vec![".claude-code-stale-1792593565991".into()],
                ..Leftovers::default()
            }
        );
        assert_eq!(names(&scope), vec![".claude-code-stale-1792593565991", "claude-code"]);
        assert!(scope
            .join(r".claude-code-stale-1792593565991\node_modules\@anthropic-ai\claude-code-win32-x64\claude.exe")
            .is_file());

        // Once nothing runs from it any more, the next update deletes it.
        drop(running);
        let report = clear_leftovers(&prefix.join("claude.cmd"), 1792593999999);
        assert_eq!(
            report,
            Leftovers {
                npm: true,
                removed: vec![".claude-code-stale-1792593565991".into()],
                ..Leftovers::default()
            }
        );
        assert_eq!(names(&scope), vec!["claude-code"]);
    }

    #[test]
    fn a_set_aside_folder_still_held_is_left_where_it_is() {
        let (_dir, prefix, scope) = npm_prefix();
        let exe = scope.join(".claude-code-stale-1792593565991").join("claude.exe");
        fs::create_dir_all(exe.parent().unwrap()).unwrap();
        let _running = Running::from(&exe);

        let report = clear_leftovers(&prefix.join("claude.cmd"), 1792593999999);

        assert_eq!(
            report,
            Leftovers {
                npm: true,
                kept: vec![".claude-code-stale-1792593565991".into()],
                ..Leftovers::default()
            }
        );
        assert!(exe.is_file());
    }

    #[test]
    fn a_rename_never_lands_on_a_folder_already_there() {
        let (_dir, prefix, scope) = npm_prefix();
        let held = scope.join(".claude-code-stale-5").join("claude.exe");
        fs::create_dir_all(held.parent().unwrap()).unwrap();
        let _first = Running::from(&held);
        let exe = scope.join(".claude-code-QrJhBTEm").join("claude.exe");
        fs::create_dir_all(exe.parent().unwrap()).unwrap();
        let _second = Running::from(&exe);

        let report = clear_leftovers(&prefix.join("claude.cmd"), 5);

        assert_eq!(report.set_aside, vec![".claude-code-stale-5-1"]);
        assert_eq!(report.kept, vec![".claude-code-stale-5"]);
        assert!(held.is_file());
        assert!(scope.join(r".claude-code-stale-5-1\claude.exe").is_file());
    }

    #[test]
    fn nothing_but_claude_code_leftovers_is_touched() {
        let (_dir, prefix, scope) = npm_prefix();
        retired(&scope, ".sdk-AbCd1234", "package.json", b"{}");
        retired(&scope, "claude-code-win32-x64", "claude.exe", b"bin");
        fs::write(scope.join(".claude-code-notes"), "a file, not a folder").unwrap();

        let report = clear_leftovers(&prefix.join("claude.cmd"), 7);

        assert_eq!(report, Leftovers { npm: true, ..Leftovers::default() });
        assert_eq!(
            names(&scope),
            vec![".claude-code-notes", ".sdk-AbCd1234", "claude-code", "claude-code-win32-x64"]
        );
    }

    #[test]
    fn the_exe_inside_the_package_finds_the_same_install() {
        let (_dir, _prefix, scope) = npm_prefix();
        retired(&scope, ".claude-code-QrJhBTEm", "README.md", b"old");

        let report = clear_leftovers(&scope.join(r"claude-code\bin\claude.exe"), 7);

        assert_eq!(report.removed, vec![".claude-code-QrJhBTEm"]);
    }

    #[test]
    fn a_command_outside_an_npm_install_touches_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join(r".local\bin");
        fs::create_dir_all(&bin).unwrap();
        fs::write(bin.join("claude.exe"), "native").unwrap();
        // Shaped like npm's leftover, but with no npm install of Claude Code beside it.
        let scope = dir.path().join(r".local\node_modules\@anthropic-ai");
        retired(&scope, ".claude-code-QrJhBTEm", "README.md", b"old");

        let report = clear_leftovers(&bin.join("claude.exe"), 7);

        assert_eq!(report, Leftovers::default());
        assert!(scope.join(r".claude-code-QrJhBTEm\README.md").is_file());
    }

    #[test]
    fn a_native_install_above_a_local_npm_install_is_not_taken_for_npm() {
        // The native installer's claude.exe in ~\.local\bin, and, separately, a local
        // `npm i @anthropic-ai/claude-code` once run in the home folder itself.
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let bin = home.join(r".local\bin");
        fs::create_dir_all(&bin).unwrap();
        fs::write(bin.join("claude.exe"), "native").unwrap();
        let scope = home.join(r"node_modules\@anthropic-ai");
        fs::create_dir_all(scope.join("claude-code")).unwrap();
        fs::write(scope.join(r"claude-code\package.json"), "{}").unwrap();
        retired(&scope, ".claude-code-QrJhBTEm", "README.md", b"old");

        let report = clear_leftovers(&bin.join("claude.exe"), 7);

        assert_eq!(report, Leftovers::default());
        assert!(scope.join(r".claude-code-QrJhBTEm\README.md").is_file());
    }

    #[test]
    fn the_report_names_its_fields_for_java() {
        let report = Leftovers {
            npm: true,
            removed: vec!["a".into()],
            set_aside: vec!["b".into()],
            kept: vec!["c".into()],
        };
        assert_eq!(report.to_json(), r#"{"kept":["c"],"npm":true,"removed":["a"],"setAside":["b"]}"#);
        assert_eq!(Leftovers::default().to_json(), r#"{"kept":[],"npm":false,"removed":[],"setAside":[]}"#);
    }
}
