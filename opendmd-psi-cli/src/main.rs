//! `psi`: Psi Formulas from the command line (opendmd-notes docs/research/psi-tooling-plan.md,
//! Phase 1; the language reference is docs/psi-spec.md, on the site at
//! /dev/psi/, its error codes docs/psi-errors.md, at /dev/psi/errors/).
//!
//!     psi check FILE.psi|- [options]     does it compile; the panel's estimates; a contact sheet
//!     psi fmt [--check|--write] FILE…|-  the canonical layout (same program, comments kept)
//!     psi build FILE.psi [--base64|-o FILE.psb]  compiled for the player (it never compiles)
//!     psi builtins --json                every name the language knows
//!     psi errors --json                  the error codes
//!
//! Install: `cargo install --path opendmd-psi-cli` (or `make psi`).

mod check;
mod json;

use opendmd_psi::errors;
use opendmd_psi::syntax::{format, tokens};
use opendmd_psi::{compile, Error};

const USAGE: &str = "usage: psi <command> …
  psi check FILE.psi|- [--out FILE.png|-] [--frames N] [--ms 0,250,…] [--scale K]
                       [--a V] [--b V] [--c V] [--clock HH:MM:SS] [--over] [--hash] [--listing] [--seed V]
                       [--levels] [--same-as OTHER.psi [--seeds N]]
      does it compile (an error's code, line, column and carets); steps and values; the panel's
      estimate on the VM and whether its JIT takes it (and why not), and whether the panel
      refuses it (exit 1); the compile's heap; its length; the compiler's notes; a PNG
      contact sheet. --levels: how often each value is
      worked out and why; --same-as: the same program and frames as another
  psi fmt [--check | --write] FILE.psi…|-
      the canonical layout: the same program, the comments kept; to stdout, or --write in
      place, or --check (exit 1 naming what isn't formatted)
  psi build FILE.psi|- [-o FILE.psb | --base64]
      the Formula compiled for the panel (which never compiles): beside it as FILE.psb,
      to -o, or --base64 for a path (`@formula?psb=…`)
  psi builtins --json    every name the language knows, with the cheat sheet's words
  psi errors --json      the error codes: code, title, sentence, what to do
  psi help";

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("{USAGE}");
        std::process::exit(2);
    }
    let cmd = args.remove(0);
    let code = match cmd.as_str() {
        "check" => check::run(args),
        "fmt" => fmt(args),
        "build" => build(args),
        "builtins" if args == ["--json"] => {
            println!("{}", json::builtins());
            0
        }
        "errors" if args == ["--json"] => {
            println!("{}", json::errors());
            0
        }
        "help" | "-h" | "--help" => {
            println!("{USAGE}");
            0
        }
        _ => {
            eprintln!("psi: unknown command `{cmd}`\n{USAGE}");
            2
        }
    };
    std::process::exit(code);
}

/// `error[E203]: line 2, column 1: <sentence>`, the line and carets under
/// the place (the code left out for a sentence with none).
pub fn render_error(e: &Error, src: &str) -> String {
    let (line, col) = e.line_col(src);
    let r = e.render(src);
    let rest = r.split_once('\n').map_or("", |x| x.1);
    let code = errors::code_of(&e.msg).map_or(String::new(), |c| format!("[{}]", errors::label(c)));
    format!("error{code}: line {line}, column {col}: {}\n{rest}\n", e.msg)
}

/// `psi build`: the compiled Formula (`opendmd_psi::psb`) beside the file,
/// to `-o`, or as base64url on stdout.
fn build(args: Vec<String>) -> i32 {
    let (mut out, mut b64, mut path) = (None, false, None);
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-o" => out = it.next(),
            "--base64" => b64 = true,
            _ if a.starts_with("--") => {
                eprintln!("psi build: unknown option {a}");
                return 2;
            }
            _ => path = Some(a),
        }
    }
    let Some(path) = path else {
        eprintln!("{USAGE}");
        return 2;
    };
    let src = check::read(&path);
    let psb = match opendmd_psi::build(&src) {
        Ok(b) => b,
        Err(e) => {
            eprint!("{path}: {}", render_error(&e, &src));
            return 1;
        }
    };
    if b64 {
        println!("{}", opendmd_psi::psb::to_base64(&psb));
        return 0;
    }
    let out = out.unwrap_or_else(|| {
        if path == "-" {
            "-".into()
        } else {
            std::path::Path::new(&path).with_extension("psb").to_string_lossy().into_owned()
        }
    });
    let r = if out == "-" {
        use std::io::Write;
        std::io::stdout().write_all(&psb)
    } else {
        std::fs::write(&out, &psb)
    };
    match r {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("psi build: {out}: {e}");
            1
        }
    }
}

/// `psi fmt`: prints, writes or checks the canonical layout, after making
/// sure the formula is still the same (see [`same`]).
fn fmt(args: Vec<String>) -> i32 {
    let (mut write, mut check, mut paths) = (false, false, Vec::new());
    for a in args {
        match a.as_str() {
            "--write" | "-w" => write = true,
            "--check" => check = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                return 0;
            }
            _ if a.starts_with("--") => {
                eprintln!("psi fmt: unknown option {a}");
                return 2;
            }
            _ => paths.push(a),
        }
    }
    if paths.is_empty() || (write && check) || (write && paths.iter().any(|p| p == "-")) {
        eprintln!("{USAGE}");
        return 2;
    }
    let mut status = 0;
    for path in &paths {
        let src = check::read(path);
        let out = match format(&src) {
            Ok(o) => o,
            Err(e) => {
                eprint!("{path}: {}", render_error(&e, &src));
                status = status.max(1);
                continue;
            }
        };
        if let Err(why) = same(&src, &out) {
            eprintln!("psi fmt: {path}: left as it is: formatting would {why} (a bug in psi fmt; please report it)");
            status = 3;
            continue;
        }
        if check {
            if out != src {
                println!("{path}");
                status = status.max(1);
            }
        } else if write {
            if out != src {
                if let Err(e) = std::fs::write(path, &out) {
                    eprintln!("psi fmt: {path}: {e}");
                    status = 2;
                }
            }
        } else {
            print!("{out}");
        }
    }
    status
}

/// The formatted text is the same formula: the same tokens (but line ends
/// just inside a `{ }`, which the compiler skips), the same program (or
/// an error both ways), the same name. Else what changed.
fn same(src: &str, out: &str) -> Result<(), &'static str> {
    if toks(src) != toks(out) {
        return Err("change its tokens");
    }
    let name = |s: &str| {
        opendmd_psi::compile_declared(s, &mut Box::<opendmd_psi::Program>::default())
            .ok()
            .and_then(|d| d.name.map(|(a, b)| s[a as usize..b as usize].to_string()))
    };
    if name(src) != name(out) {
        return Err("change its name");
    }
    match (compile(src), compile(out)) {
        (Ok(a), Ok(b)) if a == b => Ok(()),
        (Err(_), Err(_)) => Ok(()),
        _ => Err("change what it compiles to"),
    }
}

fn toks(s: &str) -> Option<Vec<(opendmd_psi::syntax::Kind, &str)>> {
    use opendmd_psi::syntax::Kind;
    let t: Vec<(Kind, &str)> = tokens(s).ok()?.into_iter().map(|(k, a, b)| (k, &s[a..b])).collect();
    Some(
        (0..t.len())
            .filter(|&k| {
                !(t[k].0 == Kind::End
                    && ((k > 0 && t[k - 1].1 == "{") || t.get(k + 1).is_some_and(|n| n.1 == "}")))
            })
            .map(|k| t[k])
            .collect(),
    )
}
