//! `psi` as a person runs it: check (ok, an error with its code), fmt
//! (stdout, --check, --write, refusing what doesn't lex), and the JSON.

use std::io::Write;
use std::process::{Command, Stdio};

fn psi(args: &[&str], stdin: &str) -> (i32, String, String) {
    let mut c = Command::new(env!("CARGO_BIN_EXE_psi"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    c.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
    let o = c.wait_with_output().unwrap();
    (o.status.code().unwrap_or(-1), String::from_utf8(o.stdout).unwrap(), String::from_utf8(o.stderr).unwrap())
}

fn tmp(name: &str, text: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("psi-cli-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    let p = d.join(name);
    std::fs::write(&p, text).unwrap();
    p
}

#[test]
fn check_says_what_a_formula_costs() {
    let (code, out, _) = psi(&["check", "-", "--out", "-"], "loop 4\nv = sin(x*3 + t/4)\nhsv(v, 1, 1)\n");
    assert_eq!(code, 0, "{out}");
    for line in ["ok: ", "panel: ~", "JIT: yes", "length: ", " B compiled", "loop 4 s at 50 fps; reads t"] {
        assert!(out.contains(line), "no `{line}` in:\n{out}");
    }
}

#[test]
fn check_gives_an_errors_code_and_place() {
    let (code, out, _) = psi(&["check", "-"], "loop 4\ntable T = 1\nT = 2\ngray(T)\n");
    assert_eq!(code, 1);
    assert!(out.starts_with("error[E203]: line 3, column 1: `T` is already the name of a table (line 2)"), "{out}");
    assert!(out.contains("\n  T = 2\n  ^\n"), "{out}");
}

#[test]
fn fmt_prints_checks_and_writes() {
    let messy = "loop 4 -- four seconds\nv=sin(x*3+t/4);w=v*0.5\nhsv( w,1,1 )";
    let tidy = "loop 4 -- four seconds\nv = sin(x*3 + t/4)\nw = v*0.5\nhsv(w, 1, 1)\n";
    let (code, out, _) = psi(&["fmt", "-"], messy);
    assert_eq!((code, out.as_str()), (0, tidy));
    let p = tmp("messy.psi", messy);
    let ps = p.to_str().unwrap();
    let (code, out, _) = psi(&["fmt", "--check", ps], "");
    assert_eq!((code, out.trim()), (1, ps));
    assert_eq!(psi(&["fmt", "--write", ps], "").0, 0);
    assert_eq!(std::fs::read_to_string(&p).unwrap(), tidy);
    assert_eq!(psi(&["fmt", "--check", ps], ""), (0, String::new(), String::new()));
    // What doesn't lex is left alone, with the error.
    let (code, _, err) = psi(&["fmt", "-"], "v = 1 ^ 2\ngray(v)\n");
    assert_eq!(code, 1);
    assert!(err.contains("error[E001]"), "{err}");
}

#[test]
fn the_json_lists_every_name_and_code() {
    let (code, out, _) = psi(&["builtins", "--json"], "");
    assert_eq!(code, 0);
    for k in ["\"functions\"", "\"palettes\"", "\"cheat_sheet\"", "\"atan2\"", "\"cycle\""] {
        assert!(out.contains(k), "no {k}");
    }
    let (code, out, _) = psi(&["errors", "--json"], "");
    assert_eq!(code, 0);
    for e in opendmd_psi::errors::ERRORS {
        assert!(out.contains(&format!("\"{}\"", opendmd_psi::errors::label(e.code))));
    }
    assert!(out.contains("https://opendmd.dev/dev/psi/#E203"));
}

#[test]
fn a_bad_command_is_refused() {
    assert_eq!(psi(&["frobnicate"], "").0, 2);
    assert_eq!(psi(&[], "").0, 2);
}

#[test]
fn check_says_why_the_jit_refuses() {
    // Nine values alive at once around helper calls (the kernel keeps eight).
    let many: String = (1..=9).map(|k| format!("v{k} = sqrt(x*{k} + y*t)\n")).collect();
    let sum: Vec<String> = (1..=9).map(|k| format!("v{k}*v{}", 10 - k)).collect();
    let (code, out, _) = psi(&["check", "-", "--out", "-"], &format!("{many}gray({})\n", sum.join(" + ")));
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("JIT: no, more values at once than the kernel's registers ("), "{out}");
}

#[test]
fn check_levels_and_notes() {
    let src = "off = px - 64\nbeam = abs(off)*2\nX = sin(t)*9\nd = px*py - 64 - X\ngray(beam/64 + d/99)\n";
    let (code, out, _) = psi(&["check", "-", "--out", "-", "--levels"], src);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("note: line 4: `px*py - 64 - X` works out `- X` for every pixel"), "{out}");
    assert!(out.contains("levels:\n"), "{out}");
    assert!(out.contains("beam  per pixel: reads `off` (line 1) ← `px`"), "{out}");
    assert!(out.contains("X     once a frame: reads `t`"), "{out}");
}

#[test]
fn check_same_as_compares_programs_and_frames() {
    let a = tmp("a.psi", "v = sin(x*3 + t/4)\nhsv(v, 1, 1)\n");
    let b = tmp("b.psi", "s = sin(t/4 + x*3)\nhsv(s, 1, 1)\n");
    let c = tmp("c.psi", "v = sin(x*3 + t/5)\nhsv(v, 1, 1)\n");
    let (a, b, c) = (a.to_str().unwrap(), b.to_str().unwrap(), c.to_str().unwrap());
    let (code, out, _) = psi(&["check", a, "--out", "-", "--same-as", a], "");
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("yes: the same program"), "{out}");
    let (code, out, _) = psi(&["check", a, "--out", "-", "--same-as", b, "--seeds", "2", "--frames", "10"], "");
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("(2 seeds × 10 over 8 s"), "{out}");
    let (code, out, _) = psi(&["check", a, "--out", "-", "--same-as", c], "");
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("same as ") && out.contains(": no: "), "{out}");
}
