//! The site's language service (feature `lang`, `make web`'s pkg-psi-lang/,
//! never packed): what the compiler saw in a formula ([`observe`]: each
//! line's share of the panel's work, the cost notes, every name read and
//! its definition) and the canonical layout ([`format`], `psi fmt`). The
//! editor on opendmd.dev imports it after a formula compiles
//! (js/fcost.js). Built with only these two exports: the device's module
//! (pkg-psi) never carries the compiler's `syntax` bookkeeping.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use opendmd_psi::syntax::{compile_observed, Defined, Level, Sink};
use opendmd_psi::{compile, Program};
use wasm_bindgen::prelude::*;

struct Seen {
    /// Each line's start (bytes), for the line an op or note is on.
    starts: Vec<usize>,
    /// Each step: where, whether per pixel, its work a frame (ns: a pixel's
    /// ×4,096, a row's ×32).
    ops: Vec<(usize, bool, u64)>,
    /// The values worked out per pixel: name, where.
    pixel: Vec<(String, usize)>,
    notes: Vec<(usize, String)>,
    reads: Vec<((usize, usize), (usize, usize))>,
}

impl Seen {
    fn line(&self, at: usize) -> usize {
        self.starts.partition_point(|&s| s <= at).max(1) - 1
    }

    /// Each line's work a frame. The steps are told before the still cache
    /// and the column run take values out of the pixel loop; their notes
    /// name what they took, so a kept value's line costs nothing a frame,
    /// one worked out once a column a 32nd.
    fn cost(&self) -> Vec<u64> {
        let mut per = vec![0u64; self.starts.len()];
        let mut div = vec![1u64; self.starts.len()];
        for (_, t) in &self.notes {
            let (d, rest) = if let Some(r) = t.strip_prefix("still cache: ") {
                (0, r)
            } else if let Some(r) = t.strip_prefix("column run: ") {
                (32, r)
            } else {
                continue;
            };
            let names = rest.split(" worked out").next().unwrap_or("");
            for n in names.split('`').skip(1).step_by(2) {
                if let Some(p) = self.pixel.iter().rev().find(|p| p.0 == n) {
                    div[self.line(p.1)] = d;
                }
            }
        }
        for &(at, px, ns) in &self.ops {
            let l = self.line(at);
            per[l] += match (px, div[l]) {
                (true, 0) => 0,
                (true, d) => ns / d,
                _ => ns,
            };
        }
        per
    }
}

impl Sink for Seen {
    fn defined(&mut self, name: &str, at: (usize, usize), _: Defined, _: u16, level: Level) {
        if level == Level::Pixel {
            self.pixel.push((name.into(), at.0));
        }
    }
    fn read(&mut self, at: (usize, usize), defined: Option<(usize, usize)>) {
        if let Some(d) = defined {
            self.reads.push((at, d));
        }
    }
    fn op(&mut self, at: (usize, usize), level: Level, _: u8, cost_ns: u32) {
        let n = match level {
            Level::Pixel => 4096,
            Level::Row => 32,
            _ => 1,
        };
        self.ops
            .push((at.0, level == Level::Pixel, u64::from(cost_ns) * n));
    }
    fn note(&mut self, at: (usize, usize), text: &str) {
        self.notes.push((self.line(at.0), text.into()));
    }
}

fn push_u(j: &mut String, mut n: u64) {
    let mut b = [0u8; 20];
    let mut k = b.len();
    loop {
        k -= 1;
        b[k] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    j.push_str(core::str::from_utf8(&b[k..]).unwrap_or("0"));
}

fn push_str(j: &mut String, t: &str) {
    j.push('"');
    for c in t.chars() {
        match c {
            '"' | '\\' => {
                j.push('\\');
                j.push(c);
            }
            c if (c as u32) < 0x20 => j.push(' '),
            c => j.push(c),
        }
    }
    j.push('"');
}

/// What compiling `src` saw, as JSON: `{"ns": the work a frame (the
/// steps' estimate, ns, the values the still cache keeps left out), "lines": [[line (0-based), per mille of it], …]
/// (lines with any work, heaviest first), "notes": [[line, sentence], …],
/// "reads": [[from, to, its definition's from, to], …]}` (offsets in
/// UTF-16 units, as JS indexes). Empty when it does not compile.
#[wasm_bindgen]
pub fn observe(src: &str) -> String {
    let mut starts = vec![0];
    starts.extend(src.match_indices('\n').map(|(k, _)| k + 1));
    let mut seen = Seen {
        starts,
        ops: Vec::new(),
        pixel: Vec::new(),
        notes: Vec::new(),
        reads: Vec::new(),
    };
    let mut prog = Box::<Program>::default();
    if compile_observed(src, &mut prog, &mut seen).is_err() {
        return String::new();
    }
    let cost = seen.cost();
    let total: u64 = cost.iter().sum();
    let mut lines: Vec<(usize, u64)> = cost.into_iter().enumerate().filter(|l| l.1 > 0).collect();
    lines.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let mut j = String::with_capacity(256 + 24 * seen.reads.len());
    j.push_str("{\"ns\":");
    push_u(&mut j, total);
    j.push_str(",\"lines\":[");
    for (k, (l, ns)) in lines.iter().enumerate() {
        if k > 0 {
            j.push(',');
        }
        j.push('[');
        push_u(&mut j, *l as u64);
        j.push(',');
        push_u(&mut j, ns * 1000 / total.max(1));
        j.push(']');
    }
    j.push_str("],\"notes\":[");
    for (k, (l, t)) in seen.notes.iter().enumerate() {
        if k > 0 {
            j.push(',');
        }
        j.push('[');
        push_u(&mut j, *l as u64);
        j.push(',');
        push_str(&mut j, t);
        j.push(']');
    }
    j.push_str("],\"reads\":[");
    for (k, (r, d)) in seen.reads.iter().enumerate() {
        if k > 0 {
            j.push(',');
        }
        for (i, b) in [r.0, r.1, d.0, d.1].into_iter().enumerate() {
            j.push(if i == 0 { '[' } else { ',' });
            push_u(&mut j, u64::from(crate::utf16_at(src, b)));
        }
        j.push(']');
    }
    j.push_str("]}");
    j
}

/// `src` in the canonical layout (`psi fmt`): only the space between
/// tokens changes, the comments kept. Empty when it cannot be laid out or
/// the result would not compile to the same program (never offered then).
#[wasm_bindgen]
pub fn format(src: &str) -> String {
    let Ok(out) = opendmd_psi::syntax::format(src) else {
        return String::new();
    };
    match (compile(src), compile(&out)) {
        (Ok(a), Ok(b)) if a == b => out,
        _ => String::new(),
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use alloc::string::ToString;

    /// A comment's text, as written (trailing spaces aside).
    fn comments(s: &str) -> Vec<String> {
        s.lines()
            .filter_map(|l| l.find("--").map(|k| l[k..].trim_end().to_string()))
            .collect()
    }

    #[test]
    fn lang_tidies_every_example_once_keeping_its_comments() {
        // (The bundled ones and the rest of the site's gallery.)
        let all = crate::examples();
        assert!(all.iter().filter(|e| e.bundled).count() >= 10 && all.len() > 30);
        for e in &all {
            let out = format(&e.src);
            assert!(!out.is_empty(), "{}: not offered", e.slug);
            assert_eq!(
                format(&out),
                out,
                "{}: tidying twice is not tidying once",
                e.slug
            );
            assert_eq!(
                comments(&out),
                comments(&e.src),
                "{}: the comments changed",
                e.slug
            );
        }
        assert_eq!(format("v = (x"), "", "an error: nothing offered");
    }

    #[test]
    fn lang_observes_where_the_work_is() {
        let src =
            "-- héavy\nloop 4\nk = sin(t/4)\nv = noise(x*8, y*8, t) * sqrt(x*x + y*y)\ngray(v*k)";
        let j = observe(src);
        assert!(j.starts_with("{\"ns\":"), "{j}");
        // The heaviest line first: line 3 (0-based), `v`.
        assert!(j.contains("\"lines\":[[3,"), "{j}");
        // `v` read on line 4 points at its definition (UTF-16 offsets: `é` is one unit).
        let at = |s: &str| src[..src.find(s).unwrap()].encode_utf16().count();
        let (def, read) = (at("v ="), at("v*k"));
        assert!(
            j.contains(&std::format!("[{read},{},{def},{}]", read + 1, def + 1)),
            "{j}"
        );
        assert_eq!(observe("v = (x"), "");
        // A value the still cache keeps costs nothing a frame: its line has no share.
        let kept = "loop 4\na = noise(x*8, y*8, t)\nc = sqrt(x*x + y*y) + atan2(y - 0.5, x - 0.5)\ngray(frac(a + c))";
        let j = observe(kept);
        let share = |l: &str| {
            j.split("\"notes\"")
                .next()
                .unwrap()
                .split(l)
                .nth(1)
                .map_or(0, |r| r.split(']').next().unwrap().parse::<u32>().unwrap())
        };
        assert!(
            j.contains("still cache: `c`") && share("[1,") > 900 && share("[2,") < 20,
            "{j}"
        );
        // Fast enough to run after every compile (the biggest bundled one).
        let road = crate::examples()
            .into_iter()
            .find(|e| e.slug == "endless-road")
            .unwrap()
            .src;
        let t = std::time::Instant::now();
        for _ in 0..10 {
            assert!(!observe(&road).is_empty());
        }
        std::eprintln!(
            "observe(Endless road): {:?} each (host, debug or release as built)",
            t.elapsed() / 10
        );
    }
}
