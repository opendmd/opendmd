//! `psi builtins --json` and `psi errors --json`: the built-in table
//! (opendmd-psi/src/builtins.rs) and the error codes (src/errors.rs) as
//! JSON, written by hand (no dependencies).

use opendmd_psi::builtins::{self, cheat};
use opendmd_psi::errors;

/// A JSON string.
fn s(v: &str) -> String {
    let mut o = String::from("\"");
    for c in v.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn list<T>(items: impl IntoIterator<Item = T>, f: impl Fn(T) -> String) -> String {
    format!("[{}]", items.into_iter().map(f).collect::<Vec<_>>().join(", "))
}

fn rows(r: &[cheat::Row]) -> String {
    list(r.iter(), |&(what, does, inserts)| {
        if inserts.is_empty() {
            format!("{{\"write\": {}, \"does\": {}}}", s(what), s(does))
        } else {
            format!("{{\"write\": {}, \"does\": {}, \"inserts\": {}}}", s(what), s(does), s(inserts))
        }
    })
}

pub fn builtins() -> String {
    let names = |v: &[&str]| list(v.iter(), |n| s(n));
    let fields = [
        ("version", builtins::VERSION.to_string()),
        (
            "values",
            list(builtins::INPUTS.iter(), |(n, _, d)| format!("{{\"name\": {}, \"means\": {}}}", s(n), s(d))),
        ),
        ("values_added_later", names(&builtins::LATER)),
        (
            "values_written_out",
            list(builtins::SUGAR.iter(), |(n, d)| format!("{{\"name\": {}, \"means\": {}}}", s(n), s(d))),
        ),
        (
            "constants",
            list(builtins::CONSTS.iter(), |(n, v)| format!("{{\"name\": {}, \"value\": {}}}", s(n), v >> 16)),
        ),
        (
            "functions",
            list(builtins::FUNCS.iter(), |(n, arity, f)| {
                let arity = match f {
                    builtins::F::Pick => "\"2+\"".to_string(),
                    builtins::F::Glyph(_) => "\"3-4\"".to_string(),
                    _ => arity.to_string(),
                };
                format!("{{\"name\": {}, \"arity\": {arity}}}", s(n))
            }),
        ),
        ("functions_added_later", names(&builtins::LATER_FUNCS)),
        (
            "colours",
            list(builtins::COLOURS.iter(), |(n, arity, _)| format!("{{\"name\": {}, \"values\": {arity}}}", s(n))),
        ),
        ("palettes", list(builtins::PALETTES.iter(), |(n, _)| s(n))),
        ("fonts", list(builtins::FONTS.iter(), |(n, _)| s(n))),
        ("blends", list(builtins::BLENDS.iter(), |(n, _)| s(n))),
        ("keywords", names(&builtins::KEYWORDS)),
        ("declarations", names(&builtins::DECLARATIONS)),
        ("context_words", names(&builtins::CONTEXT_WORDS)),
        ("draw_primitives", names(&builtins::PRIMITIVES)),
        (
            "cheat_sheet",
            format!(
                "{{\"values\": {}, \"functions\": {}, \"draw\": {}, \"colours\": {}, \"declarations\": {}}}",
                rows(cheat::VARS),
                rows(cheat::FUNCS),
                rows(cheat::DRAW),
                rows(cheat::COLS),
                rows(cheat::DECLS)
            ),
        ),
    ];
    format!(
        "{{\n{}\n}}",
        fields.iter().map(|(k, v)| format!("  {}: {v}", s(k))).collect::<Vec<_>>().join(",\n")
    )
}

pub fn errors() -> String {
    let items: Vec<String> = errors::ERRORS
        .iter()
        .map(|e| {
            format!(
                "  {{\"code\": {}, \"title\": {}, \"sentence\": {}, \"fix\": {}, \"url\": {}}}",
                s(&errors::label(e.code)),
                s(e.title),
                s(e.sentence),
                s(e.fix),
                s(&format!("https://opendmd.dev/dev/psi/errors/#{}", errors::label(e.code)))
            )
        })
        .collect();
    format!("[\n{}\n]", items.join(",\n"))
}
