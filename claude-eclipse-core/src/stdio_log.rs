//! Debug lines for this process's stderr, which never panic or hold up their caller.
//!
//! The process is Eclipse, and its stderr is whatever whoever started Eclipse handed
//! it. A shortcut or a console gives it nothing or a console, and `eprintln!` is fine.
//! A launcher that starts Eclipse as a child and then never reads its output (PDE
//! Tools' "Launch New Workspace" does) gives it a pipe that first fills up, so a write
//! WAITS, and later loses its reader, so a write FAILS, and `eprintln!` panics on a
//! failed write. Either one, on a chat's reader thread, took that tab down.
//!
//! So the lines go through a queue to one writer thread of their own. The caller only
//! ever queues: when the queue is full the line is dropped, and when stderr fails the
//! writer stops and later lines are dropped. Only the writer can wait on stderr.

use std::io::Write;
use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::OnceLock;

/// Lines that may wait for the writer. Past this, new ones are dropped.
const QUEUE_LINES: usize = 1024;

/// A queue of lines and the thread that writes them to one sink.
pub(crate) struct LineWriter {
    queue: SyncSender<String>,
}

impl LineWriter {
    /// Starts the writer for `sink`. If its thread cannot be started, every line is
    /// dropped, as if the sink had failed.
    pub(crate) fn new<W: Write + Send + 'static>(sink: W, queue_lines: usize) -> LineWriter {
        let (queue, lines) = sync_channel::<String>(queue_lines);
        let _ = std::thread::Builder::new()
            .name("claude-stderr-log".into())
            .spawn(move || {
                let mut sink = sink;
                for mut line in lines {
                    line.push('\n');
                    // One write per line, as `eprintln!` did, so lines from here and
                    // anything else on stderr do not interleave mid-line.
                    if sink.write_all(line.as_bytes()).and_then(|()| sink.flush()).is_err() {
                        break; // a failed stderr stays failed; the rest are dropped
                    }
                }
            });
        LineWriter { queue }
    }

    /// Queues `line`, or drops it. Never waits.
    pub(crate) fn line(&self, line: String) {
        let _ = self.queue.try_send(line);
    }
}

/// Writes `line` to stderr, through the queue.
pub(crate) fn submit(line: String) {
    static STDERR: OnceLock<LineWriter> = OnceLock::new();
    STDERR.get_or_init(|| LineWriter::new(std::io::stderr(), QUEUE_LINES)).line(line);
}

/// Sends panic messages through the same queue. Rust's own hook writes them straight
/// to stderr, which would wait on the same full pipe. Installed once, from
/// `JNI_OnLoad`, never in tests: the hook is process-wide.
pub(crate) fn install_panic_hook() {
    // Only formatting and queueing: a hook that panicked would abort Eclipse.
    std::panic::set_hook(Box::new(|info| submit(info.to_string())));
}

/// `eprintln!` for the plugin's own diagnostics: same arguments, same line on stderr,
/// but through [`submit`], so it can never panic or hold up its caller.
#[macro_export]
macro_rules! elog {
    ($($arg:tt)*) => {
        $crate::stdio_log::submit(::std::format!($($arg)*))
    };
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Child, Command, Stdio};
    use std::sync::mpsc::{channel, Receiver, Sender};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    /// Set on the copy of this test binary that the tests below start as a child.
    const CHILD_ENV: &str = "CLAUDE_ECLIPSE_STDIO_LOG_CHILD";
    const CHILD_TEST: &str = "stdio_log::tests::child_logs_after_go";
    const WAIT: Duration = Duration::from_secs(20);

    /// The child's half. In a normal run it does nothing. Started by the tests below,
    /// it waits for a line on stdin, so its stderr is already in the state the test
    /// wants, then logs about 1 MB and returns.
    #[test]
    fn child_logs_after_go() {
        if std::env::var_os(CHILD_ENV).is_none() {
            return;
        }
        let mut go = String::new();
        std::io::stdin().read_line(&mut go).ok();
        let filler = "x".repeat(100);
        for n in 0..10_000 {
            crate::elog!("[stdio-log test] line {n} {filler}");
        }
    }

    /// This test binary again, running only the child's half, with `--nocapture` so
    /// the harness does not swallow the lines before they reach the real stderr.
    fn spawn_child() -> Child {
        Command::new(std::env::current_exe().unwrap())
            .args([CHILD_TEST, "--exact", "--nocapture", "--test-threads=1"])
            .env(CHILD_ENV, "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    }

    fn release(child: &mut Child) {
        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(b"go\n").unwrap();
    }

    /// The child's exit status, or `None` if it was still running after `WAIT`
    /// (it is killed then).
    fn wait_for(child: &mut Child) -> Option<std::process::ExitStatus> {
        let started = Instant::now();
        while started.elapsed() < WAIT {
            if let Some(status) = child.try_wait().unwrap() {
                return Some(status);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        child.kill().ok();
        child.wait().ok();
        None
    }

    /// Eclipse started through PDE Tools' "Launch New Workspace": stderr is a pipe
    /// whose reader has gone away. Logging must neither panic nor fail the caller.
    #[test]
    fn logging_survives_a_stderr_whose_reader_is_gone() {
        let mut child = spawn_child();
        drop(child.stderr.take()); // the read end closes here, before any line is written
        release(&mut child);
        let status = wait_for(&mut child).expect("the child hung");
        assert!(status.success(), "logging to a closed stderr failed the caller: {status}");
    }

    /// The same launch before the reader goes: a pipe nobody reads, which fills up.
    /// Logging must never wait for room in it.
    #[test]
    fn logging_never_waits_on_a_stderr_nobody_reads() {
        let mut child = spawn_child();
        let unread = child.stderr.take(); // held open, never read
        release(&mut child);
        let status = wait_for(&mut child);
        drop(unread);
        let status = status.expect("logging blocked on a full stderr");
        assert!(status.success(), "{status}");
    }

    /// A stderr that works shows every line, whole, in order: what `eprintln!` showed.
    #[test]
    fn a_working_stderr_gets_every_line_in_order() {
        let seen = Arc::new(Mutex::new(Vec::<u8>::new()));
        let writer = LineWriter::new(Shared(Arc::clone(&seen)), QUEUE_LINES);
        for n in 0..100 {
            writer.line(format!("line {n}"));
        }
        let expected: String = (0..100).map(|n| format!("line {n}\n")).collect();
        let started = Instant::now();
        while seen.lock().unwrap().len() < expected.len() && started.elapsed() < WAIT {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(String::from_utf8(seen.lock().unwrap().clone()).unwrap(), expected);
    }

    /// A sink that fails is written to once, then left alone; the caller never sees it.
    #[test]
    fn a_failing_sink_is_given_up_on_quietly() {
        let (attempts, attempted) = channel();
        let writer = LineWriter::new(Failing(attempts), 4);
        for n in 0..10_000 {
            writer.line(format!("line {n}"));
        }
        attempted.recv_timeout(WAIT).expect("the writer never wrote");
        std::thread::sleep(Duration::from_millis(50));
        assert!(attempted.try_recv().is_err(), "kept writing to a failed sink");
    }

    /// A sink that never takes a line: the caller still never waits.
    #[test]
    fn a_stuck_sink_never_holds_up_the_caller() {
        let (_unblock, gate) = channel::<()>();
        let writer = LineWriter::new(Stuck(gate), 4);
        let started = Instant::now();
        for n in 0..10_000 {
            writer.line(format!("line {n}"));
        }
        assert!(started.elapsed() < Duration::from_secs(2), "took {:?}", started.elapsed());
    }

    struct Shared(Arc<Mutex<Vec<u8>>>);
    impl Write for Shared {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    struct Failing(Sender<()>);
    impl Write for Failing {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            self.0.send(()).ok();
            Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// Blocks on its first write until the test ends and drops the other end.
    struct Stuck(Receiver<()>);
    impl Write for Stuck {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            let _ = self.0.recv();
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
}
