# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Print the web app's size, raw and gzipped, and enforce the budget.

    uv run tools/web-size.py opendmd-web/www

Budget (gzipped bytes — what actually crosses the WiFi, since the firmware
serves `<file>.gz` in preference): the wasm module plus its wasm-bindgen JS
glue (`pkg/`, generated, grows with the export list) <= 100 KB; the
hand-written JS + CSS <= 48 KB (40 KB until Follow RePlayOS's Settings card,
which had ~0.6 KB left). The Studio ("Make art") is loaded only when it
opens and has lines of its own: its wasm + glue (`pkg-art/`) <= 150 KB, and
`js/studio.js` + `studio.css` <= 24 KB — neither counts against the lines
above. The name index for "Find art for a game" (`art/*.json`, one file
fetched per search) is printed, with no budget. Raw sizes are printed for
information. A file with no `.gz` beside it counts at its raw size. Exits 1
when over budget.
"""
import sys
from pathlib import Path

WASM_GZ_BUDGET = 100 * 1024
JS_CSS_GZ_BUDGET = 48 * 1024
ART_WASM_GZ_BUDGET = 150 * 1024
STUDIO_GZ_BUDGET = 24 * 1024
STUDIO = {"js/studio.js", "studio.css"}


def main() -> int:
    root = Path(sys.argv[1] if len(sys.argv) > 1 else "opendmd-web/www")
    files = sorted(p for p in root.rglob("*") if p.is_file() and p.suffix != ".gz"
                   and not p.name.startswith("."))
    totals: dict[str, list[int]] = {}
    print(f"{'file':<40} {'raw':>9} {'gz':>9}")
    for p in files:
        raw = p.stat().st_size
        gz_path = p.with_name(p.name + ".gz")
        gz = gz_path.stat().st_size if gz_path.exists() else raw
        rel = p.relative_to(root).as_posix()
        if rel.startswith("pkg/"):
            kind = "wasm+glue"
        elif rel.startswith("pkg-art/"):
            kind = "art wasm"
        elif rel in STUDIO:
            kind = "studio"
        elif rel.startswith("art/"):
            kind = "art index"
            t = totals.setdefault(kind, [0, 0])
            t[0] += raw
            t[1] += gz
            continue
        else:
            kind = {".js": "js+css", ".css": "js+css"}.get(p.suffix, "other")
        t = totals.setdefault(kind, [0, 0])
        t[0] += raw
        t[1] += gz
        print(f"{str(p.relative_to(root)):<40} {raw:>9,} {gz:>9,}")
    print()
    ok = True
    for kind, budget in (("wasm+glue", WASM_GZ_BUDGET), ("js+css", JS_CSS_GZ_BUDGET),
                         ("art wasm", ART_WASM_GZ_BUDGET), ("studio", STUDIO_GZ_BUDGET),
                         ("art index", None), ("other", None)):
        raw, gz = totals.get(kind, [0, 0])
        verdict = "" if budget is None else ("ok" if gz <= budget else "OVER BUDGET")
        limit = "" if budget is None else f"/ {budget // 1024} KB"
        print(f"{kind:<10} raw {raw:>9,}           gz {gz:>9,} {limit:<10} {verdict}")
        ok &= budget is None or gz <= budget
    all_raw = sum(t[0] for t in totals.values())
    all_gz = sum(t[1] for t in totals.values())
    print(f"{'total':<10} raw {all_raw:>9,}           gz {all_gz:>9,}")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
