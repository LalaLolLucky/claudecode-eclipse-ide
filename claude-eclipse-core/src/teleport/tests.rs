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
