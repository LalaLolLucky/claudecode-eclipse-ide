//! Helpers shared by the crate's unit tests.

use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

/// The variable [`crate::session::dirs_home`] reads.
#[cfg(windows)]
const HOME_VAR: &str = "USERPROFILE";
#[cfg(not(windows))]
const HOME_VAR: &str = "HOME";

/// One lock for every test that changes an environment variable. The environment
/// is process-wide and tests run on parallel threads, so a per-module lock would
/// still let one module's test see another's value.
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Holds [`ENV_LOCK`] for the life of a test, and puts back every variable it
/// changed when dropped, including when the test fails.
pub(crate) struct EnvGuard {
    saved: Vec<(String, Option<OsString>)>,
    _lock: MutexGuard<'static, ()>,
}

impl EnvGuard {
    /// Takes the lock. Hold the guard for the whole test.
    pub(crate) fn lock() -> Self {
        // A test that failed while holding the lock poisons it; its guard restored
        // the environment regardless, so later tests can proceed.
        let lock = ENV_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        EnvGuard { saved: Vec::new(), _lock: lock }
    }

    /// Sets `key` until the guard is dropped.
    pub(crate) fn set(&mut self, key: &str, value: impl AsRef<OsStr>) {
        self.save(key);
        std::env::set_var(key, value);
    }

    /// Unsets `key` until the guard is dropped.
    pub(crate) fn remove(&mut self, key: &str) {
        self.save(key);
        std::env::remove_var(key);
    }

    /// Points the home directory at `home` until the guard is dropped.
    pub(crate) fn set_home(&mut self, home: &Path) {
        self.set(HOME_VAR, home);
    }

    /// Records a variable's value the first time it is touched.
    fn save(&mut self, key: &str) {
        if !self.saved.iter().any(|(k, _)| k == key) {
            self.saved.push((key.to_string(), std::env::var_os(key)));
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, value) in self.saved.drain(..).rev() {
            match value {
                Some(v) => std::env::set_var(&key, v),
                None => std::env::remove_var(&key),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // Each test owns its variable, so no other test reads or writes it.

    #[test]
    fn restores_a_variable_that_was_unset() {
        const KEY: &str = "CLAUDE_ECLIPSE_TEST_SUPPORT_UNSET";
        std::env::remove_var(KEY);
        {
            let mut env = EnvGuard::lock();
            env.set(KEY, "during");
            assert_eq!(std::env::var(KEY).as_deref(), Ok("during"));
        }
        assert!(std::env::var_os(KEY).is_none(), "unset again after the guard dropped");
    }

    #[test]
    fn restores_the_value_from_before_its_first_change() {
        const KEY: &str = "CLAUDE_ECLIPSE_TEST_SUPPORT_FIRST";
        std::env::set_var(KEY, "before");
        {
            let mut env = EnvGuard::lock();
            env.set(KEY, "first");
            env.set(KEY, "second");
            env.remove(KEY);
        }
        assert_eq!(std::env::var(KEY).as_deref(), Ok("before"));
        std::env::remove_var(KEY);
    }
}
