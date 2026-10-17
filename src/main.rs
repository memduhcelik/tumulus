use std::io::{self, Write};
use std::process;

fn main() {
    // The runtime ignores SIGPIPE and turns a closed pipe into an error.
    // jq dies on that signal instead, which is what `head` expects.
    reset_sigpipe();
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());
    let code = match tumulus::run_cli(&mut out) {
        Ok(code) => code,
        Err(err) => {
            let _ = out.flush();
            eprintln!("tumulus: {err}");
            1
        }
    };
    if let Err(err) = out.flush() {
        eprintln!("tumulus: {err}");
        process::exit(1);
    }
    if code != 0 {
        process::exit(code);
    }
}

fn reset_sigpipe() {
    #[cfg(unix)]
    unsafe {
        extern "C" {
            fn signal(sig: i32, handler: usize) -> usize;
        }
        // 13 is SIGPIPE. 0 is SIG_DFL.
        signal(13, 0);
    }
}
