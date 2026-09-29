use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

static LOCK_FILE_PATH: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();

fn state() -> &'static Mutex<Option<PathBuf>> {
    LOCK_FILE_PATH.get_or_init(|| Mutex::new(None))
}

// ---------------------------------------------------------------------------
// Stale lock file cleanup
//
// Claude CLI reads every *.lock file in ~/.claude/ide/ and tries to connect
// to each IDE it finds.  When an Eclipse instance crashes without removing
// its lock file, or multiple live instances are open, Claude may attempt
// connections with mismatched auth tokens and fail.  We scan on startup and
// remove any lock file whose PID is no longer alive.
// ---------------------------------------------------------------------------

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn OpenProcess(desired_access: u32, inherit_handle: i32, pid: u32) -> isize;
    fn CloseHandle(handle: isize) -> i32;
}

fn is_pid_alive(pid: u32) -> bool {
    #[cfg(windows)]
    {
        // PROCESS_QUERY_LIMITED_INFORMATION = 0x1000
        let h = unsafe { OpenProcess(0x1000, 0, pid) };
        if h == 0 { return false; }
        unsafe { CloseHandle(h) };
        true
    }
    #[cfg(target_os = "linux")]
    { std::path::Path::new(&format!("/proc/{}", pid)).exists() }
    #[cfg(not(any(windows, target_os = "linux")))]
    { let _ = pid; true } // macOS etc — conservatively assume alive
}

/// Remove any *.lock files in `dir` whose recorded PID is no longer running.
fn remove_stale_lock_files(dir: &Path) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("lock") {
            continue;
        }
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                if let Some(pid) = json.get("pid").and_then(|v| v.as_u64()) {
                    if !is_pid_alive(pid as u32) {
                        let _ = std::fs::remove_file(&path);
                    }
                }
            }
        }
    }
}

/// Writes ~/.claude/ide/<port>.lock with the JSON content Claude CLI expects.
///
/// `project_paths_json` must be a JSON array string, e.g. `["/path/a","/path/b"]`.
pub fn write(port: u16, auth_token: &str, workspace_root: &str, project_paths_json: &str) {
    let dir = lock_file_dir();

    // Remove stale lock files from dead processes before advertising ourselves.
    remove_stale_lock_files(&dir);

    if let Err(e) = std::fs::create_dir_all(&dir) {
        if crate::is_debug() {
            eprintln!("lock_file: create_dir_all {:?}: {}", dir, e);
        }
        return;
    }

    let path = dir.join(format!("{}.lock", port));

    // Parse the project paths array so we can include it in the JSON object.
    let folders: serde_json::Value = serde_json::from_str(project_paths_json)
        .unwrap_or_else(|_| serde_json::json!([]));

    let pid = std::process::id();

    let content = serde_json::json!({
        "port":            port,
        "authToken":       auth_token,
        "version":         "0.2.0",
        "ideName":         "Eclipse",
        "pid":             pid,
        "workspaceFolder": workspace_root,
        "workspaceFolders": folders
    })
    .to_string();

    match std::fs::write(&path, content) {
        Ok(_) => {
            *state().lock().unwrap() = Some(path);
        }
        Err(e) => {
            if crate::is_debug() {
                eprintln!("lock_file: write {:?}: {}", path, e);
            }
        }
    }
}

/// Removes the lock file created by the last call to `write`.
pub fn remove() {
    let mut guard = state().lock().unwrap();
    if let Some(path) = guard.take() {
        if let Err(e) = std::fs::remove_file(&path) {
            if crate::is_debug() {
                eprintln!("lock_file: remove {:?}: {}", path, e);
            }
        }
    }
}

/// Removes all lock files EXCEPT the one for the given port.
/// Call this before launching Claude CLI to ensure it only finds our server.
pub fn remove_other_lock_files(our_port: u16) {
    let dir = lock_file_dir();
    let our_filename = format!("{}.lock", our_port);

    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
            if filename.ends_with(".lock") && filename != our_filename {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
}

fn lock_file_dir() -> PathBuf {
    if let Ok(config_dir) = std::env::var("CLAUDE_CONFIG_DIR") {
        if !config_dir.is_empty() {
            return PathBuf::from(config_dir).join("ide");
        }
    }
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".claude").join("ide")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::EnvGuard;
    use serde_json::json;

    /// Beyond every OS's PID range, so any check that really looks reports it dead.
    const DEAD_PID: u32 = u32::MAX;

    /// Every test that writes or deletes lock files points CLAUDE_CONFIG_DIR at a
    /// throwaway folder first: the real ~/.claude/ide holds the live IDEs' lock files.
    fn isolated_lock_dir(env: &mut EnvGuard) -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        env.set("CLAUDE_CONFIG_DIR", tmp.path());
        tmp
    }

    #[test]
    fn claude_config_dir_moves_the_lock_directory() {
        let mut env = EnvGuard::lock();
        let tmp = isolated_lock_dir(&mut env);
        assert_eq!(lock_file_dir(), tmp.path().join("ide"));
    }

    #[test]
    fn without_claude_config_dir_the_lock_directory_is_under_home() {
        let mut env = EnvGuard::lock();
        env.set("CLAUDE_CONFIG_DIR", ""); // empty counts as unset
        env.set("USERPROFILE", "profile-home");
        env.set("HOME", "unix-home");
        assert_eq!(
            lock_file_dir(),
            PathBuf::from("profile-home").join(".claude").join("ide"),
            "USERPROFILE is checked before HOME, on every platform"
        );
        env.remove("USERPROFILE");
        assert_eq!(lock_file_dir(), PathBuf::from("unix-home").join(".claude").join("ide"));
        env.remove("HOME");
        assert_eq!(lock_file_dir(), PathBuf::from(".").join(".claude").join("ide"));
    }

    #[test]
    fn write_produces_the_lock_file_claude_reads_and_remove_deletes_it() {
        let mut env = EnvGuard::lock();
        let tmp = isolated_lock_dir(&mut env);

        write(48123, "secret-token", "C:/ws", r#"["C:/ws/a","C:/ws/b"]"#);
        let path = tmp.path().join("ide").join("48123.lock");
        let lock: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(lock["port"], 48123);
        assert_eq!(lock["authToken"], "secret-token");
        assert_eq!(lock["ideName"], "Eclipse");
        assert_eq!(lock["version"], "0.2.0");
        assert_eq!(lock["pid"], std::process::id());
        assert_eq!(lock["workspaceFolder"], "C:/ws");
        assert_eq!(lock["workspaceFolders"], json!(["C:/ws/a", "C:/ws/b"]));

        remove();
        assert!(!path.exists(), "remove deletes the file write created");
    }

    #[test]
    fn malformed_project_paths_become_an_empty_list() {
        let mut env = EnvGuard::lock();
        let tmp = isolated_lock_dir(&mut env);

        write(48124, "t", "C:/ws", "not json");
        let path = tmp.path().join("ide").join("48124.lock");
        let lock: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(lock["workspaceFolders"], json!([]));
        remove();
    }

    #[test]
    fn this_process_counts_as_alive() {
        assert!(is_pid_alive(std::process::id()));
    }

    #[test]
    fn stale_cleanup_removes_only_lock_files_of_dead_processes() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let put = |name: &str, body: String| std::fs::write(dir.join(name), body).unwrap();
        put("dead.lock", json!({ "pid": DEAD_PID }).to_string());
        put("live.lock", json!({ "pid": std::process::id() }).to_string());
        put("dead.txt", json!({ "pid": DEAD_PID }).to_string());
        put("garbage.lock", "not json".to_string());
        put("no-pid.lock", json!({ "port": 1 }).to_string());

        remove_stale_lock_files(dir);

        for kept in ["live.lock", "dead.txt", "garbage.lock", "no-pid.lock"] {
            assert!(dir.join(kept).exists(), "{kept} must be left alone");
        }
        #[cfg(windows)]
        assert!(!dir.join("dead.lock").exists());
        #[cfg(target_os = "linux")]
        assert!(!dir.join("dead.lock").exists());
        // Elsewhere is_pid_alive cannot check and treats every PID as alive.
        #[cfg(not(any(windows, target_os = "linux")))]
        assert!(dir.join("dead.lock").exists());
    }

    #[test]
    fn remove_other_lock_files_keeps_ours_and_anything_not_a_lock() {
        let mut env = EnvGuard::lock();
        let tmp = isolated_lock_dir(&mut env);
        let dir = tmp.path().join("ide");
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["100.lock", "200.lock", "notes.txt"] {
            std::fs::write(dir.join(name), "{}").unwrap();
        }

        remove_other_lock_files(100);

        assert!(dir.join("100.lock").exists());
        assert!(dir.join("notes.txt").exists());
        assert!(!dir.join("200.lock").exists());
    }
}
