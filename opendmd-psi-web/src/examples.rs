//! The examples gallery (`opendmd-web/www/formulas.json`) read on the host,
//! for the tests and the contact sheet (`examples/sheet.rs`). The file is
//! an array of flat objects — `name`, `desc`, `src` strings and optional
//! `a`, `b`, `c` numbers — so a small reader does.

/// One example.
#[derive(Clone, Debug, Default)]
pub struct Example {
    pub name: String,
    pub desc: String,
    pub src: String,
    /// The sliders it starts with, Q16.16.
    pub a: i32,
    pub b: i32,
    pub c: i32,
}

/// A JSON string starting at `s[0] == '"'`: its value and the rest.
fn string(s: &str) -> (String, &str) {
    let mut out = String::new();
    let mut it = s[1..].char_indices();
    while let Some((i, c)) = it.next() {
        match c {
            '"' => return (out, &s[i + 2..]),
            '\\' => match it.next().map(|x| x.1) {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('u') => {
                    let hex: String = (0..4).filter_map(|_| it.next().map(|x| x.1)).collect();
                    out.extend(u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32));
                }
                Some(c) => out.push(c),
                None => break,
            },
            c => out.push(c),
        }
    }
    panic!("unterminated string in formulas.json")
}

/// Every example in the file, in order.
pub fn examples(json: &str) -> Vec<Example> {
    let mut all = Vec::new();
    let mut s = json;
    let mut cur: Option<Example> = None;
    while let Some(c) = s.chars().next() {
        match c {
            '{' => {
                cur = Some(Example::default());
                s = &s[1..];
            }
            '}' => {
                all.extend(cur.take());
                s = &s[1..];
            }
            '"' => {
                let (key, rest) = string(s);
                let rest = rest
                    .trim_start()
                    .strip_prefix(':')
                    .expect("a key")
                    .trim_start();
                let e = cur.as_mut().expect("a key outside an object");
                if rest.starts_with('"') {
                    let (v, r) = string(rest);
                    match key.as_str() {
                        "name" => e.name = v,
                        "desc" => e.desc = v,
                        "src" => e.src = v,
                        _ => {}
                    }
                    s = r;
                } else {
                    let end = rest.find([',', '}']).unwrap_or(rest.len());
                    let v: f64 = rest[..end].trim().parse().expect("a number");
                    let q = (v * 65536.0).round() as i32;
                    match key.as_str() {
                        "a" => e.a = q,
                        "b" => e.b = q,
                        "c" => e.c = q,
                        _ => {}
                    }
                    s = &rest[end..];
                }
            }
            _ => s = &s[c.len_utf8()..],
        }
    }
    all
}
