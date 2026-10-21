//! Where a run reads its arguments and standard input, and where it writes
//! errors. The binary leaves this unset. A test fills it and calls the same
//! program in this process, so `stop` unwinds instead of exiting.

use std::cell::RefCell;
use std::io::{self, Write};
use std::process;

thread_local! {
    static CAPTURE: RefCell<Option<Capture>> = const { RefCell::new(None) };
}

struct Capture {
    /// `args[0]` is the program name.
    args: Vec<String>,
    stdin: Vec<u8>,
    stdin_at: usize,
    stderr: Vec<u8>,
    /// The version line. The filter writes to the caller's buffer instead.
    stdout_extra: Vec<u8>,
}

/// Passed to `panic` so a test can keep running after the program stops.
pub(crate) struct Stop(pub i32);

pub(crate) fn capturing() -> bool {
    CAPTURE.with(|slot| slot.borrow().is_some())
}

pub(crate) fn begin(args: &[String], stdin: &[u8]) {
    CAPTURE.with(|slot| {
        *slot.borrow_mut() = Some(Capture {
            args: args.to_vec(),
            stdin: stdin.to_vec(),
            stdin_at: 0,
            stderr: Vec::new(),
            stdout_extra: Vec::new(),
        });
    });
}

/// Stderr written so far, and any version line. Clears the capture.
pub(crate) fn take() -> (Vec<u8>, Vec<u8>) {
    CAPTURE.with(|slot| match slot.borrow_mut().take() {
        Some(cap) => (cap.stderr, cap.stdout_extra),
        None => (Vec::new(), Vec::new()),
    })
}

pub(crate) fn args() -> Vec<String> {
    let owned = CAPTURE.with(|slot| slot.borrow().as_ref().map(|cap| cap.args.clone()));
    owned.unwrap_or_else(|| std::env::args().collect())
}

pub(crate) fn read_stdin(buf: &mut [u8]) -> io::Result<usize> {
    let taken = CAPTURE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let cap = slot.as_mut()?;
        let rest = &cap.stdin[cap.stdin_at..];
        let n = rest.len().min(buf.len());
        buf[..n].copy_from_slice(&rest[..n]);
        cap.stdin_at += n;
        Some(n)
    });
    match taken {
        Some(n) => Ok(n),
        None => io::Read::read(&mut io::stdin(), buf),
    }
}

pub(crate) fn read_stdin_to_end(buf: &mut Vec<u8>) -> io::Result<()> {
    let taken = CAPTURE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(cap) = slot.as_mut() else {
            return false;
        };
        buf.extend_from_slice(&cap.stdin[cap.stdin_at..]);
        cap.stdin_at = cap.stdin.len();
        true
    });
    if taken {
        return Ok(());
    }
    io::Read::read_to_end(&mut io::stdin(), buf).map(|_| ())
}

/// A captured run has a pipe, not a terminal.
pub(crate) fn stdout_is_terminal() -> bool {
    if capturing() {
        false
    } else {
        io::IsTerminal::is_terminal(&io::stdout())
    }
}

pub(crate) fn write_stdout_line(line: &str) {
    let diverted = CAPTURE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(cap) = slot.as_mut() else {
            return false;
        };
        cap.stdout_extra.extend_from_slice(line.as_bytes());
        cap.stdout_extra.push(b'\n');
        true
    });
    if !diverted {
        println!("{line}");
    }
}

pub(crate) fn write_stderr_bytes(bytes: &[u8]) {
    let diverted = CAPTURE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(cap) = slot.as_mut() else {
            return false;
        };
        cap.stderr.extend_from_slice(bytes);
        true
    });
    if diverted {
        return;
    }
    let mut err = io::stderr().lock();
    let _ = err.write_all(bytes);
    let _ = err.flush();
}

pub(crate) fn emit_stderr(args: std::fmt::Arguments<'_>, newline: bool) {
    if capturing() {
        CAPTURE.with(|slot| {
            let mut slot = slot.borrow_mut();
            let Some(cap) = slot.as_mut() else {
                return;
            };
            let _ = cap.stderr.write_fmt(args);
            if newline {
                cap.stderr.push(b'\n');
            }
        });
        return;
    }
    let mut err = io::stderr().lock();
    let _ = err.write_fmt(args);
    if newline {
        let _ = err.write_all(b"\n");
    }
    let _ = err.flush();
}

/// End this run. A captured run unwinds with [`Stop`]. The binary exits.
pub(crate) fn stop(code: i32) -> ! {
    if capturing() {
        std::panic::panic_any(Stop(code));
    }
    process::exit(code);
}

#[macro_export]
macro_rules! eprint {
    ($($arg:tt)*) => {
        $crate::host::emit_stderr(::std::format_args!($($arg)*), false)
    };
}

#[macro_export]
macro_rules! eprintln {
    ($($arg:tt)*) => {
        $crate::host::emit_stderr(::std::format_args!($($arg)*), true)
    };
}
