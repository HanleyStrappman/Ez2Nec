use std::process::ExitCode;

use ez2nec::{dump, ez};

const USAGE: &str = "\
Usage: ez2nec --dump FILE.ez...

  --dump       Print each decoded model, and anything in the file that the
               decoder places but doesn't understand or can't account for.
  -h, --help   Show this help.
  --version    Show the version.

Writing NEC decks is not implemented yet.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut want_dump = false;
    let mut files = Vec::new();
    for a in &args {
        match a.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            "--version" => {
                println!("ez2nec {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            "--dump" => want_dump = true,
            s if s.starts_with('-') => {
                eprintln!("ez2nec: unknown option {s}\n\n{USAGE}");
                return ExitCode::from(2);
            }
            s => files.push(s),
        }
    }
    if !want_dump || files.is_empty() {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    }

    let mut failed = false;
    for (i, path) in files.iter().enumerate() {
        if i > 0 {
            println!();
        }
        println!("File:       {path}");
        let data = match std::fs::read(path) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("ez2nec: {path}: {e}");
                failed = true;
                continue;
            }
        };
        match ez::read_ez(&data) {
            Ok(file) => print!("{}", dump::dump(&file)),
            Err(e) => {
                eprintln!("ez2nec: {path}: {e}");
                failed = true;
            }
        }
    }
    if failed { ExitCode::from(1) } else { ExitCode::SUCCESS }
}
