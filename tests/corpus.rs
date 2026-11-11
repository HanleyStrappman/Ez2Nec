//! Decodes every `.ez` file under the directories named in `EZ2NEC_CORPUS`
//! (separated like `PATH`). Without the variable the test does nothing, since
//! no EZNEC files are in the repository.
//!
//!     EZ2NEC_CORPUS=~/models:~/more cargo test --test corpus -- --nocapture

use std::path::{Path, PathBuf};

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect(&p, out);
        } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("ez")) {
            out.push(p);
        }
    }
}

#[test]
fn corpus_decodes_with_nothing_unaccounted() {
    let Some(dirs) = std::env::var_os("EZ2NEC_CORPUS") else {
        eprintln!("EZ2NEC_CORPUS is not set; skipping");
        return;
    };
    let mut files = Vec::new();
    for d in std::env::split_paths(&dirs) {
        collect(&d, &mut files);
    }
    files.sort();
    assert!(!files.is_empty(), "no .ez files under EZ2NEC_CORPUS");

    let mut problems = Vec::new();
    for f in &files {
        let data = std::fs::read(f).expect("readable");
        match ez2nec::ez::read_ez(&data) {
            Err(e) => problems.push(format!("{}: {e}", f.display())),
            Ok(file) => {
                ez2nec::dump::dump(&file);
                for u in &file.report.unclaimed {
                    problems.push(format!(
                        "{}: unaccounted bytes at {:#06x}: {:02x?}",
                        f.display(),
                        u.offset,
                        u.bytes
                    ));
                }
            }
        }
    }
    eprintln!("{} files, {} problems", files.len(), problems.len());
    assert!(problems.is_empty(), "\n{}", problems.join("\n"));
}
