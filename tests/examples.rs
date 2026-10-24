//! Every example program in `tests/examples` must get through each compiler
//! phase built so far with no diagnostics. As later phases land, they are
//! added here.

use muc::lexer::{self, TokenKind};
use muc::source::SourceMap;
use std::path::PathBuf;

fn example_files() -> Vec<PathBuf> {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/examples");
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("mu"))
        .collect();
    files.sort();
    assert!(
        files.len() >= 5,
        "expected at least 5 example programs, found {}",
        files.len()
    );
    files
}

fn load(path: &PathBuf) -> (SourceMap, muc::source::FileId) {
    let text = std::fs::read_to_string(path).unwrap();
    let mut sources = SourceMap::new();
    let file = sources.add(path.display().to_string(), text);
    (sources, file)
}

#[test]
fn examples_lex_cleanly() {
    for path in example_files() {
        let (sources, file) = load(&path);
        let mut diags = Vec::new();
        let tokens = lexer::lex(file, &sources.get(file).text, &mut diags);
        let rendered: String = diags.iter().map(|d| d.render(&sources)).collect();
        assert!(diags.is_empty(), "{}:\n{rendered}", path.display());
        assert_eq!(tokens.last().map(|t| &t.kind), Some(&TokenKind::Eof));
    }
}

#[test]
fn examples_parse_cleanly() {
    for path in example_files() {
        let (sources, file) = load(&path);
        let mut diags = Vec::new();
        let parsed = muc::parse_project(&sources, &[file], &mut diags);
        let rendered: String = diags.iter().map(|d| d.render(&sources)).collect();
        assert!(diags.is_empty(), "{}:\n{rendered}", path.display());
        assert!(
            !parsed[0].items.is_empty(),
            "{} has no items",
            path.display()
        );
    }
}
