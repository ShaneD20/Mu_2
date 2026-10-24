//! The `muc` command-line driver.

use muc::source::SourceMap;
use muc::{diag, lexer, sexp};
use std::process::ExitCode;

const USAGE: &str = "\
usage: muc <command> <file.mu>

commands:
  lex      print the token stream after newline layout
  parse    print the syntax tree as S-expressions";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (command, path) = match args.as_slice() {
        [command, path] => (command.as_str(), path.as_str()),
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };

    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) => {
            eprintln!("error: cannot read `{path}`: {err}");
            return ExitCode::from(2);
        }
    };
    let mut sources = SourceMap::new();
    let file = sources.add(path, text);
    let mut diags = Vec::new();

    match command {
        "lex" => {
            let tokens = lexer::lex(file, &sources.get(file).text, &mut diags);
            let src = sources.get(file);
            for token in &tokens {
                let (line, col) = src.line_col(token.span.lo as usize);
                println!(
                    "{line:>4}:{col:<3} {:<11} {}",
                    token.kind.category(),
                    token.kind
                );
            }
        }
        "parse" => {
            let files = muc::parse_project(&sources, &[file], &mut diags);
            for node in sexp::file(&files[0]) {
                println!("{}\n", node.pretty(100));
            }
        }
        _ => {
            eprintln!("unknown command `{command}`\n\n{USAGE}");
            return ExitCode::from(2);
        }
    }

    for d in &diags {
        eprintln!("{}", d.render(&sources));
    }
    if diag::has_errors(&diags) {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
