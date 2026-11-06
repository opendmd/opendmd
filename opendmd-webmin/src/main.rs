//! `opendmd-webmin SRC OUT FILE...`: minify each of the web app's scripts and
//! stylesheets on its own, `SRC/FILE` to `OUT/FILE` (make web, through
//! tools/web-minify.py; docs/notes/web-app.md "Minified build").
//!
//! No bundling: every file stays a file at its own path, its import
//! specifiers as written, so the module graph is the one the sources have.
//!
//! - **JS** (`.js`): oxc, parsed as a module (`SourceType::mjs()`), the
//!   compressor at its defaults with an ES2022 target (top-level await is the
//!   newest syntax the sources need; nothing newer is written), and the
//!   mangler on **nested scopes only** (`top_level: false`): top-level names
//!   and every export keep their spelling, so nothing another module (or the
//!   site's pages) imports by name changes. Property names are never mangled.
//!   oxc's assumptions (crates/oxc_minifier/docs/ASSUMPTIONS.md: no `with`,
//!   `document.all`, direct `eval`, `Function.prototype.toString`, reliance on
//!   function or class `.name`, side-effecting coercions or built-in patches)
//!   were checked against the app on 2026-11-05: it relies on none of them.
//!   A parse error fails the build, never ships a half-minified file.
//! - **CSS** (`.css`): lightningcss, minified, no browser targets (nothing
//!   lowered or prefixed); `url()`s and `@font-face` are kept as written.
//!
//! The output's comments are dropped, legal comments (`/*! */`, `@license`)
//! included: the app has none, and its notices are licenses.txt.

use std::path::Path;
use std::process::ExitCode;

use lightningcss::stylesheet::{MinifyOptions, ParserOptions, PrinterOptions, StyleSheet};
use oxc_allocator::Allocator;
use oxc_codegen::{Codegen, CodegenOptions, CommentOptions};
use oxc_compat::EngineTargets;
use oxc_minifier::{CompressOptions, MangleOptions, Minifier, MinifierOptions};
use oxc_parser::Parser;
use oxc_span::SourceType;

/// The newest syntax the output may use (see the module docs).
const TARGET: &str = "es2022";

fn js(name: &str, src: &str) -> Result<String, String> {
    let alloc = Allocator::default();
    let ret = Parser::new(&alloc, src, SourceType::mjs()).parse();
    if ret.diagnostics.has_errors() {
        let errs: Vec<String> = ret.diagnostics.errors().map(|e| e.to_string()).collect();
        return Err(format!("{name}: does not parse: {}", errs.join("; ")));
    }
    let mut program = ret.program;
    let target = EngineTargets::from_target(TARGET)?;
    let options = MinifierOptions {
        mangle: Some(MangleOptions {
            top_level: Some(false),
            ..MangleOptions::default()
        }),
        mangle_properties: None,
        compress: Some(CompressOptions {
            target,
            ..CompressOptions::default()
        }),
    };
    let ret = Minifier::new(options).minify(&alloc, &mut program);
    let out = Codegen::new()
        .with_options(CodegenOptions {
            minify: true,
            comments: CommentOptions::disabled(),
            ..CodegenOptions::default()
        })
        .with_scoping(ret.scoping)
        .build(&program);
    Ok(out.code)
}

fn css(name: &str, src: &str) -> Result<String, String> {
    let mut sheet = StyleSheet::parse(
        src,
        ParserOptions {
            filename: name.to_string(),
            ..ParserOptions::default()
        },
    )
    .map_err(|e| format!("{name}: does not parse: {e}"))?;
    sheet
        .minify(MinifyOptions::default())
        .map_err(|e| format!("{name}: {e}"))?;
    let out = sheet
        .to_css(PrinterOptions {
            minify: true,
            ..PrinterOptions::default()
        })
        .map_err(|e| format!("{name}: {e}"))?;
    Ok(out.code)
}

fn run(src: &Path, out: &Path, files: &[String]) -> Result<(), String> {
    for f in files {
        let text = std::fs::read_to_string(src.join(f)).map_err(|e| format!("{f}: {e}"))?;
        let min = match Path::new(f).extension().and_then(|e| e.to_str()) {
            Some("js") => js(f, &text)?,
            Some("css") => css(f, &text)?,
            _ => return Err(format!("{f}: neither .js nor .css")),
        };
        let dst = out.join(f);
        if let Some(d) = dst.parent() {
            std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
        }
        std::fs::write(&dst, min).map_err(|e| format!("{}: {e}", dst.display()))?;
    }
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        eprintln!("usage: opendmd-webmin SRC OUT FILE...   (FILE relative to SRC, .js or .css)");
        return ExitCode::from(2);
    }
    match run(Path::new(&args[0]), Path::new(&args[1]), &args[2..]) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("opendmd-webmin: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{css, js};

    #[test]
    fn exports_top_level_names_and_specifiers_are_kept() {
        let src = "import { emit as send } from './util.js';\n\
                   // a comment\n\
                   const LABELS = { a: 'All files' };\n\
                   export function playlistLabel(path) { const local = path + '!'; return LABELS[local] ?? local; }\n\
                   export class Preview { get path() { return this.p; } }\n\
                   export const later = () => import('./smooth.js');\n\
                   const at = (n) => import(`./js/${n}.js`);\n\
                   send(at);\n";
        let out = js("t.js", src).unwrap();
        for kept in [
            "./util.js",
            "./smooth.js",
            "./js/${",
            "export function playlistLabel(",
            "export class Preview",
            "LABELS",
            "later",
            "get path()",
        ] {
            assert!(out.contains(kept), "{kept:?} not in {out}");
        }
        assert!(
            !out.contains("a comment") && !out.contains("local"),
            "{out}"
        );
    }

    #[test]
    fn top_level_await_and_utf8_survive() {
        let out = js("t.js", "const web = await import('./pkg/x.js');\nexport const T = 'aren’t … ›';\nexport default web;\n").unwrap();
        assert!(
            out.contains("await import(") && out.contains("aren’t … ›"),
            "{out}"
        );
    }

    #[test]
    fn a_syntax_error_fails() {
        assert!(js("bad.js", "export const = 1;").is_err());
    }

    #[test]
    fn css_keeps_urls_and_custom_properties() {
        let out = css("t.css", "/* c */\n@font-face { font-family: Silkscreen; src: url(fonts/silkscreen.woff2) format('woff2'); }\n:root { --acc: #f0a030; }\n#now-path { color: var(--acc); }\n").unwrap();
        for kept in [
            "url(fonts/silkscreen.woff2)",
            "--acc:#f0a030",
            "#now-path",
            "var(--acc)",
        ] {
            assert!(out.contains(kept), "{kept:?} not in {out}");
        }
        assert!(!out.contains("/*"));
    }
}
