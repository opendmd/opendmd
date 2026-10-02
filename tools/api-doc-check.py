# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Check docs/api.md against the routes firmware/src/http.rs registers.

    uv run tools/api-doc-check.py [docs/api.md] [firmware/src/http.rs]

Run by `make web`. Both ways:

- every `/api/` route registered with `server.route("<uri>", Method::X, …)`
  (or `server.route_json`)
  has a `### METHOD /api/…` heading in docs/api.md, and every such heading
  is a registered route with that method;
- every `/api/…` path mentioned anywhere in docs/api.md is a route
  (exactly, or under a `…/*` wildcard).

`<name>` / `&lt;name&gt;` in a heading stands for a wildcard's `*`. Exits 1
with the differences.
"""
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent


def main() -> int:
    md_path = Path(sys.argv[1]) if len(sys.argv) > 1 else REPO / "docs" / "api.md"
    rs_path = Path(sys.argv[2]) if len(sys.argv) > 2 else REPO / "firmware" / "src" / "http.rs"
    rs = rs_path.read_text(encoding="utf-8")
    md = md_path.read_text(encoding="utf-8")
    routes = {(m, u) for u, m in re.findall(r'server\.route(?:_json)?\(\s*"(/api/[^"]*)",\s*Method::(\w+)', rs)}
    routes = {(m.upper(), u) for m, u in routes}
    uris = {u for _, u in routes}

    documented = set()
    for meth, path in re.findall(r"^### ([A-Z]+(?:, [A-Z]+)*) (/api/\S*)", md, re.M):
        path = re.sub(r"(?:<|&lt;)[^/]*$", "*", path)
        for m in meth.split(", "):
            documented.add((m, path))

    def real(path: str) -> bool:
        return path in uris or any(u.endswith("/*") and path.startswith(u[:-1]) and len(path) > len(u) - 1
                                   for u in uris)

    problems = []
    for m, u in sorted(routes - documented):
        problems.append(f"route {m} {u} (http.rs) has no '### {m} {u}' heading in docs/api.md")
    for m, u in sorted(documented - routes):
        problems.append(f"docs/api.md documents {m} {u}, which http.rs does not register")
    mentioned = set()
    for p, placeholder in re.findall(r"(/api/[A-Za-z0-9_./-]*)(<|&lt;)?", md):
        # `/api/files/<path>` names the wildcard itself.
        mentioned.add(p + "*" if placeholder else p.rstrip("."))
    for p in sorted(mentioned):
        if not real(p):
            problems.append(f"docs/api.md mentions {p}, which is not a route")
    for p in problems:
        print(f"api-doc-check: {p}")
    if problems:
        return 1
    print(f"api-doc-check: {len(routes)} routes documented, {len(mentioned)} /api/ paths mentioned, all real")
    return 0


if __name__ == "__main__":
    sys.exit(main())
