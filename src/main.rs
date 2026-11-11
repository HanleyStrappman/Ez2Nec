use std::path::Path;
use std::process::ExitCode;

use ez2nec::nec::{Engine, Options, write_deck};
use ez2nec::{dump, ez};

const USAGE: &str = "\
Usage: ez2nec --engine nec2|nec4|nec5 [-o FILE.nec] FILE.ez
       ez2nec --dump FILE.ez...

  --engine E   Write a NEC deck for NEC-2, NEC-4.2 or NEC-5. The deck goes to
               standard output unless -o names a file. What the engine can't
               express exactly is noted in the deck's CM lines and on
               standard error.
  -o FILE      Write the deck to FILE.
  --dump       Print each decoded model, and anything in the file that the
               decoder places but doesn't understand or can't account for.
  -h, --help   Show this help.
  --version    Show the version.";

fn usage_error(message: &str) -> ExitCode {
    eprintln!("ez2nec: {message}\n\n{USAGE}");
    ExitCode::from(2)
}

fn read(path: &str) -> Result<ez::EzFile, String> {
    let data = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    ez::read_ez(&data).map_err(|e| format!("{path}: {e}"))
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let mut want_dump = false;
    let mut engine = None;
    let mut output = None;
    let mut files = Vec::new();
    while let Some(a) = args.next() {
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
        if engine.is_some() || output.is_some() {
            return usage_error("--dump can't be combined with --engine or -o");
        }
        let mut failed = false;
        for (i, path) in files.iter().enumerate() {
            if i > 0 {
                println!();
            }
            println!("File:       {path}");
            match read(path) {
                Ok(file) => print!("{}", dump::dump(&file)),
                Err(e) => {
                    eprintln!("ez2nec: {e}");
                    failed = true;
                }
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
    let deck = match write_deck(&file.model, &Options { engine, source_name }) {
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
        None => print!("{}", deck.text),
    }
    ExitCode::SUCCESS
}
