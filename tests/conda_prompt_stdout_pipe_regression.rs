//! Regression guard for the `cx create` / `cx env create` stdout-piping bug.
//!
//! Conda asks for confirmation by writing a prompt to **stdout without a newline**, then
//! calling `stdin.readline()` (see `conda/plugins/reporter_backends/console.py`). The old
//! `cx` path piped conda stdout and consumed it with `BufRead::lines()`, which blocks until
//! a newline. The child blocks on stdin at the same time, so the prompt never reaches the
//! user and input appears swallowed.
//!
//! This test does not run the real `cx` binary; it reproduces that **conda-shaped** stdout/stdin
//! pattern with a shell child. If someone reverts to always piping `create` on a TTY, this
//! documents why that regresses.

#[cfg(unix)]
mod unix {
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Command, Stdio};
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant};

    use rstest::rstest;

    /// Same sequencing as conda's console reporter: partial line on stdout, block on stdin, then
    /// flush a newline (conda does `sys.stdout.write("\\n")` after `readline()` returns).
    const CHILD: &str = "printf 'Proceed (y/n)? '; read _; printf '\\n'; echo after";

    #[derive(Clone, Copy, Debug)]
    enum FirstLineMode {
        /// Matches `BufRead::read_line` usage.
        ReadLine,
        /// Matches `run_conda_filtered`'s `reader.lines()` loop.
        LinesIterator,
    }

    #[rstest]
    #[case::read_line(FirstLineMode::ReadLine)]
    #[case::lines_iterator(FirstLineMode::LinesIterator)]
    fn line_reader_gets_no_stdout_line_until_stdin_unblocks_child(#[case] mode: FirstLineMode) {
        let mut child = Command::new("/bin/sh")
            .arg("-c")
            .arg(CHILD)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn /bin/sh");

        let mut stdin = child.stdin.take().expect("child stdin");
        let stdout = child.stdout.take().expect("child stdout");

        let (tx, rx) = mpsc::channel();
        let reader = thread::spawn(move || {
            let start = Instant::now();
            let mut buf = BufReader::new(stdout);
            let line = match mode {
                FirstLineMode::ReadLine => {
                    let mut line = String::new();
                    buf.read_line(&mut line)
                        .expect("read_line from piped child stdout");
                    line
                }
                FirstLineMode::LinesIterator => buf
                    .lines()
                    .next()
                    .expect("lines iterator should yield")
                    .expect("first line from piped child stdout"),
            };
            tx.send((start.elapsed(), line))
                .expect("send timing + line to main thread");
        });

        const DELAY: Duration = Duration::from_millis(150);
        thread::sleep(DELAY);

        stdin
            .write_all(b"y\n")
            .expect("answer child's read as the user would");
        stdin.flush().expect("flush stdin");

        let (elapsed, line) = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("reader thread should finish after stdin unblocks child");

        reader.join().expect("reader thread panicked");

        let status = child.wait().expect("wait child");
        assert!(status.success(), "child should exit 0");

        assert!(
            elapsed >= DELAY.saturating_sub(Duration::from_millis(30)),
            "line-based reader should block until stdin is answered (conda-style prompt); got {:?}",
            elapsed
        );

        assert_eq!(
            line.trim_end(),
            "Proceed (y/n)?",
            "prompt stayed buffered until stdin unblocked; first complete line is the prompt, not earlier output"
        );
    }
}

#[cfg(not(unix))]
#[test]
fn conda_prompt_stdout_pipe_pattern_skipped_on_non_unix() {
    // The regression reproducer uses `/bin/sh`; behavior is documented in the module comment.
}
