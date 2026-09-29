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
