use std::io::{ErrorKind, Write};
use std::path::Path;
use std::process::ExitCode;

use ez2nec::nec::{Engine, Options, write_deck};
use ez2nec::{dump, ez};

const USAGE: &str = "\
Usage: ez2nec --engine nec2|nec4|nec5 [--sweep] [-o FILE.nec] FILE.ez
       ez2nec --dump FILE.ez...

  --engine E   Write a NEC deck for NEC-2, NEC-4.2 or NEC-5. The deck goes to
               standard output unless -o names a file. What the engine can't
               express exactly is noted in the deck's CM lines and on
               standard error.
  --sweep      Run the frequency sweep stored in the model instead of its one
               frequency. Loads and networks whose values depend on frequency
               are written again for each frequency.
  -o FILE      Write the deck to FILE.
  --dump       Print each decoded model, and anything in the file that the
               decoder places but doesn't understand or can't account for.
  -h, --help   Show this help.
  --version    Show the version.";

fn usage_error(message: &str) -> ExitCode {
    eprintln!("ez2nec: {message}\n\n{USAGE}");
    ExitCode::from(2)
}

/// Why writing to standard output stopped.
#[derive(Debug, PartialEq)]
enum Stop {
    /// The reader went away, as in `ez2nec --dump FILE.ez | head`: the rest
    /// of the output isn't wanted, which is not an error.
    ReaderGone,
    /// Any other write error, already reported on standard error.
    Failed,
}

impl Stop {
    fn exit_code(self, failed_before: bool) -> ExitCode {
        match self {
            Stop::ReaderGone if !failed_before => ExitCode::SUCCESS,
            _ => ExitCode::from(1),
        }
    }
}

fn emit_to(out: &mut impl Write, text: &str) -> Result<(), Stop> {
    match out.write_all(text.as_bytes()).and_then(|()| out.flush()) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::BrokenPipe => Err(Stop::ReaderGone),
        Err(e) => {
            eprintln!("ez2nec: standard output: {e}");
            Err(Stop::Failed)
        }
    }
}

/// Writes to standard output. `print!` would panic when the reader has gone.
fn emit(text: &str) -> Result<(), Stop> {
    emit_to(&mut std::io::stdout().lock(), text)
}

fn read(path: &str) -> Result<ez::EzFile, String> {
    let data = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    ez::read_ez(&data).map_err(|e| format!("{path}: {e}"))
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let mut want_dump = false;
    let mut sweep = false;
    let mut engine = None;
    let mut output = None;
    let mut files = Vec::new();
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => {
                return match emit(&format!("{USAGE}\n")) {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(stop) => stop.exit_code(false),
                };
            }
            "--version" => {
                return match emit(&format!("ez2nec {}\n", env!("CARGO_PKG_VERSION"))) {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(stop) => stop.exit_code(false),
                };
            }
            "--dump" => want_dump = true,
            "--sweep" => sweep = true,
            "--engine" => {
                let Some(name) = args.next() else {
                    return usage_error("--engine needs a name");
                };
                let Some(e) = Engine::parse(&name) else {
                    return usage_error(&format!("unknown engine {name}"));
                };
                engine = Some(e);
            }
            "-o" => {
                let Some(path) = args.next() else {
                    return usage_error("-o needs a file name");
                };
                output = Some(path);
            }
            s if s.starts_with('-') => return usage_error(&format!("unknown option {s}")),
            _ => files.push(a),
        }
    }
    if files.is_empty() {
        return usage_error("no input file");
    }

    if want_dump {
        if engine.is_some() || output.is_some() || sweep {
            return usage_error("--dump can't be combined with --engine, --sweep or -o");
        }
        let mut failed = false;
        for (i, path) in files.iter().enumerate() {
            let mut text = String::new();
            if i > 0 {
                text.push('\n');
            }
            text.push_str(&format!("File:       {path}\n"));
            let file = read(path);
            if let Ok(file) = &file {
                text.push_str(&dump::dump(file));
            }
            if let Err(stop) = emit(&text) {
                return stop.exit_code(failed);
            }
            if let Err(e) = file {
                eprintln!("ez2nec: {e}");
                failed = true;
            }
        }
        return if failed { ExitCode::from(1) } else { ExitCode::SUCCESS };
    }

    let Some(engine) = engine else {
        return usage_error("choose an engine with --engine, or use --dump");
    };
    if files.len() != 1 {
        return usage_error("--engine converts one file at a time");
    }
    let path = &files[0];
    let file = match read(path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("ez2nec: {e}");
            return ExitCode::from(1);
        }
    };
    let source_name = Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned());
    let deck = match write_deck(
        &file.model,
        &Options {
            engine,
            source_name,
            sweep,
        },
    ) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("ez2nec: {path}: {e}");
            return ExitCode::from(1);
        }
    };
    if !file.report.unclaimed.is_empty() {
        eprintln!(
            "ez2nec: {path}: {} run(s) of bytes in the file are not accounted for; see --dump",
            file.report.unclaimed.len()
        );
    }
    for note in &deck.notes {
        eprintln!("ez2nec: note: {note}");
    }
    match output {
        Some(out) => {
            if let Err(e) = std::fs::write(&out, &deck.text) {
                eprintln!("ez2nec: {out}: {e}");
                return ExitCode::from(1);
            }
        }
        None => {
            if let Err(stop) = emit(&deck.text) {
                return stop.exit_code(false);
            }
        }
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    /// A writer whose every write fails with the given error kind.
    struct Failing(ErrorKind);

    impl Write for Failing {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::from(self.0))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn emit_writes_the_text() {
        let mut out = Vec::new();
        assert_eq!(emit_to(&mut out, "CE\n"), Ok(()));
        assert_eq!(out, b"CE\n");
    }

    #[test]
    fn a_closed_pipe_stops_quietly() {
        let stop = emit_to(&mut Failing(ErrorKind::BrokenPipe), "CE\n");
        assert_eq!(stop, Err(Stop::ReaderGone));
        assert_eq!(Stop::ReaderGone.exit_code(false), ExitCode::SUCCESS);
        assert_eq!(Stop::ReaderGone.exit_code(true), ExitCode::from(1));
    }

    #[test]
    fn other_write_errors_fail() {
        let stop = emit_to(&mut Failing(ErrorKind::PermissionDenied), "CE\n");
        assert_eq!(stop, Err(Stop::Failed));
        assert_eq!(Stop::Failed.exit_code(false), ExitCode::from(1));
    }
}
