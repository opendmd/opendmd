// The gallery checker's rules in the Worker: opendmd-gallery's
// `check_file` (opendmd-gallery/src/lib.rs), line for line where the
// Worker can, with the site's own Psi (src/psi.ts) as the compiler. The
// public repo's pull-request check is still the gate; this is the second
// look tools/site.py used to take at every build, now at every sync
// (src/gallery.ts). site-worker-test runs both over the same files and
// compares every verdict, sentence and field.

import { compiled, type Compiled } from "./psi";

export const MAX_FILE = 36 * 1024;
export const MAX_PSB = 4500;
export const MAX_NAME = 40;
export const MAX_DESCRIPTION = 120;
export const MAX_AUTHOR = 40;
export const MAX_SLUG = 48;
export const MAX_US = 10_000;
export const LICENCES = ["CC0-1.0", "CC-BY-4.0"];
// Keep in step with opendmd-gallery's RESERVED (and the public repo's CODEOWNERS).
const RESERVED = ["opendmd", "duggan"];
const MAX_MARKS = 2;

/** An entry that passed: what opendmd-gallery's `check --json` writes. */
export interface Entry {
  slug: string;
  name: string;
  about: string;
  author: string;
  licence: string;
  bundle: boolean;
  look: [string, number] | null;
  controls: string[];
  src: string;
  loop_ms: number;
  inputs: number;
  /** What the panel takes a frame: opendmd-gallery's `Program::tier_us(false)`, the JIT's where it takes the Formula (info[11] when info[15] is 1), else the VM's with the still cache (info[14]). */
  us: number;
  /** The VM's (info[14]): "without the JIT" (the same as `us` when the JIT does not take it). */
  vm_us: number;
  /** The Formula compiled for the panel: bytes, and the format's version. */
  psb_len: number;
  abi: number;
}

export type Verdict = { ok: true; entry: Entry; program: Compiled } | { ok: false; problems: string[] };

/** A slug on its own (opendmd-gallery `check_slug`). */
export function checkSlug(slug: string): string | null {
  if (!/^[a-z0-9-]*$/.test(slug) || !slug || slug.length > MAX_SLUG || slug.startsWith("-") || slug.endsWith("-") || slug.includes("--")) {
    return `the file name must be lower-case letters, digits and single hyphens, at most ${MAX_SLUG} long, then .psi (like rainbow-rings.psi)`;
  }
  if (slug === "readme" || slug === "index") return "that file name is taken";
  return null;
}

function between(u: number, a: number, b: number): boolean {
  return u >= a && u <= b;
}

/** opendmd-gallery `bad_char`. */
export function badChar(u: number, inFormula: boolean): string | null {
  if (u === 0x09 && inFormula) return null;
  if (between(u, 0, 0x1f) || between(u, 0x7f, 0x9f)) return "a control character";
  if (between(u, 0x202a, 0x202e) || between(u, 0x2066, 0x2069) || u === 0x200e || u === 0x200f || u === 0x061c) {
    return "a character that changes the text's direction";
  }
  if (
    u === 0x00ad || u === 0x034f || u === 0x115f || u === 0x1160 || u === 0x17b4 || u === 0x17b5 || between(u, 0x180b, 0x180e) ||
    between(u, 0x200b, 0x200d) || between(u, 0x2060, 0x2065) || between(u, 0x206a, 0x206f) || u === 0x2800 || u === 0x3164 ||
    between(u, 0xfe00, 0xfe0f) || u === 0xfeff || u === 0xffa0 || between(u, 0xe0000, 0xe0fff)
  ) {
    return "an invisible character";
  }
  if (between(u, 0x2000, 0x200a) || between(u, 0x2028, 0x2029) || u === 0x202f || u === 0x205f || u === 0x3000 || u === 0x00a0) {
    return "an unusual space or line break";
  }
  if (between(u, 0xfff0, 0xffff)) return "a special character";
  if (between(u, 0xe000, 0xf8ff) || between(u, 0xf0000, 0x10ffff)) return "a private-use character";
  return null;
}

const MARKS: [number, number][] = [
  [0x0300, 0x036f], [0x0483, 0x0489], [0x0591, 0x05bd], [0x05bf, 0x05bf], [0x05c1, 0x05c2], [0x05c4, 0x05c5], [0x05c7, 0x05c7],
  [0x0610, 0x061a], [0x064b, 0x065f], [0x0670, 0x0670], [0x06d6, 0x06dc], [0x06df, 0x06e4], [0x06e7, 0x06e8], [0x06ea, 0x06ed],
  [0x0900, 0x0903], [0x093a, 0x094f], [0x0e31, 0x0e31], [0x0e34, 0x0e3a], [0x0e47, 0x0e4e], [0x1ab0, 0x1aff], [0x1dc0, 0x1dff],
  [0x20d0, 0x20ff], [0x302a, 0x302f], [0x3099, 0x309a], [0xfe20, 0xfe2f],
];

/** opendmd-gallery `is_mark`: a combining mark. */
export function isMark(u: number): boolean {
  return MARKS.some(([a, b]) => u >= a && u <= b);
}

const LOOKALIKE: Record<string, string> = {
  "а": "a", "в": "b", "с": "c", "ԁ": "d", "е": "e", "ё": "e", "һ": "h", "і": "i", "ї": "i", "ј": "j", "к": "k", "м": "m", "н": "h",
  "о": "o", "р": "p", "ԛ": "q", "ѕ": "s", "т": "t", "ц": "u", "ѵ": "v", "ԝ": "w", "х": "x", "у": "y", "α": "a", "β": "b", "ε": "e",
  "η": "n", "ι": "i", "κ": "k", "μ": "m", "ν": "v", "ο": "o", "ρ": "p", "τ": "t", "υ": "u", "χ": "x", "ɡ": "g", "ı": "i", "ℓ": "l",
  "0": "o", "1": "l", "ñ": "n", "ç": "c",
};

/** opendmd-gallery `skeleton`: a rough "looks like" form of a name. */
export function skeleton(s: string): string {
  let out = "";
  for (const ch of s) {
    let u = ch.codePointAt(0)!;
    if (between(u, 0xff01, 0xff5e)) u -= 0xfee0;
    if (isMark(u)) continue;
    for (let c of String.fromCodePoint(u).toLowerCase()) {
      const k = c.codePointAt(0)!;
      c = LOOKALIKE[c] ??
        (between(k, 0xe0, 0xe5) ? "a" : between(k, 0xe8, 0xeb) ? "e" : between(k, 0xec, 0xef) ? "i" : between(k, 0xf2, 0xf6) ? "o" : between(k, 0xf9, 0xfc) ? "u" : c);
      if (/^[a-z0-9]$/.test(c)) out += c;
    }
  }
  return out;
}

const chars = (s: string) => Array.from(s).length;

function field(problems: string[], what: string, v: string, max: number): void {
  const n = chars(v);
  if (n === 0) problems.push(`the ${what} is empty`);
  else if (n > max) problems.push(`the ${what} is ${n} characters: ${max} at most`);
  if (v.includes("<") || v.includes(">")) problems.push(`the ${what} may not contain < or >`);
  if (v !== v.trim() || v.includes("  ")) problems.push(`the ${what} has extra spaces`);
}

function header(line: string | undefined, keys: string[]): string | null {
  if (line === undefined || !line.startsWith("-- ")) return null;
  const rest = line.slice(3);
  for (const k of keys) {
    if (rest.startsWith(k) && rest[k.length] === ":") return rest.slice(k.length + 1).trim();
  }
  return null;
}

const hex4 = (u: number) => u.toString(16).toUpperCase().padStart(4, "0");

/** A contribution, as opendmd-gallery `check_file` (untrusted) checks it. */
export function checkFile(slug: string, bytes: Uint8Array): Verdict {
  if (bytes.length > MAX_FILE) return { ok: false, problems: [`the file is ${bytes.length} bytes: ${MAX_FILE} at most`] };
  let text: string;
  try {
    text = new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(bytes);
  } catch {
    return { ok: false, problems: ["the file is not plain text (UTF-8)"] };
  }
  if (text.includes("\0")) return { ok: false, problems: ["the file is not plain text"] };
  const problems: string[] = [];
  const lines = text.split("\n");
  const bundle = lines[2] !== undefined && lines[2].trimEnd() === "-- bundle";
  const skip = bundle ? 3 : 2;
  const seen = new Set<string>();
  const crlf = text.includes("\r\n");
  if (crlf) problems.push("the file has Windows line endings (CRLF): save it with plain (LF) line endings");
  lines.forEach((l, i) => {
    let marks = 0;
    for (const c of l) {
      const u = c.codePointAt(0)!;
      marks = isMark(u) ? marks + 1 : 0;
      if (marks === MAX_MARKS + 1) problems.push(`line ${i + 1}: more than ${MAX_MARKS} accents on one letter`);
      if (crlf && c === "\r") continue;
      const why = badChar(u, i >= skip);
      if (why && !seen.has(why)) {
        seen.add(why);
        problems.push(`line ${i + 1}: ${why} (U+${hex4(u)})`);
      }
    }
  });
  const bad = checkSlug(slug);
  if (bad) problems.push(bad);
  const author = header(lines[0], ["author"]), licence = header(lines[1], ["licence", "license"]);
  if (author === null || licence === null) {
    problems.push(
      header(lines[0], ["name"]) !== null
        ? "the header is two lines now, `-- author: …` and `-- licence: CC0-1.0`: the name and the description are the formula's own first lines, `name \"…\"` and `about \"…\"` (and `-- values:` is the `control` lines' defaults)"
        : "the file must start with two lines: `-- author: …`, `-- licence: CC0-1.0`, then the formula, starting `name \"…\"` and `about \"…\"`",
    );
    return { ok: false, problems };
  }
  if (bundle) problems.push("`-- bundle` is for the examples OpenDMD ships: leave it out");
  field(problems, "author", author, MAX_AUTHOR);
  if (!LICENCES.includes(licence)) problems.push(`the licence must be one of ${LICENCES.join(", ")}`);
  // The formula: everything after the header.
  let offset = 0;
  for (let k = 0; k < skip; k++) {
    const nl = text.indexOf("\n", offset);
    if (nl < 0) {
      offset = text.length;
      break;
    }
    offset = nl + 1;
  }
  const src = text.slice(offset);
  if (src.split("\n").every((l) => { const t = l.trim(); return !t || t.startsWith("--") || t.startsWith("//"); })) {
    problems.push("there is no formula after the header");
    return { ok: false, problems };
  }
  const c = compiled(src);
  if (!c.ok) {
    const line = skip + (src.slice(0, c.at).match(/\n/g)?.length ?? 0) + 1;
    problems.push(`line ${line}: the formula doesn't compile: ${c.error}`);
    return { ok: false, problems };
  }
  const { name, about } = c.meta;
  if (name !== null) field(problems, "name", name, MAX_NAME);
  else problems.push("the formula needs a name: a first line like `name \"Rainbow rings\"`");
  if (about !== null) field(problems, "about line", about, MAX_DESCRIPTION);
  else problems.push("the formula needs a line about it, after its name: `about \"Rings rippling out from the middle\"`");
  for (const [what, v] of [["name", name ?? ""], ["author", author]] as const) {
    if (RESERVED.includes(skeleton(v))) problems.push(`the ${what} can't be OpenDMD's or a maintainer's`);
  }
  // opendmd-gallery: Program::tier_us(false), as the panel plays it (the JIT's estimate where its kernel takes the Formula: info[15]).
  const us = c.info[15] ? c.info[11] : c.info[14];
  if (us > MAX_US) {
    problems.push(`the formula is too heavy for the panel: about ${(us / 1000).toFixed(1)} ms a frame, ${(MAX_US / 1000).toFixed(0)} ms at most`);
  }
  // The Formula as compiled for the panel (a `@formula?psb=…` link carries it).
  if (c.psb === 0) problems.push("the formula could not be prepared for the panel");
  else if (c.psb > MAX_PSB) {
    problems.push(`the formula is too long for the panel to take in a link: ${c.psb} bytes once prepared, ${MAX_PSB} at most; cut tables or repeated steps (comments and spaces cost nothing)`);
  }
  if (problems.length) return { ok: false, problems };
  // (controls() writes the amount as a number: 100 for `look` alone)
  const look = c.meta.look ? ([c.meta.look[0], Math.round(c.meta.look[1])] as [string, number]) : null;
  return {
    ok: true,
    program: c,
    entry: {
      slug, name: name!, about: about!, author, licence, bundle, look,
      controls: c.meta.controls.map((k) => k.label),
      src, loop_ms: c.info[4], inputs: c.info[5], us,
      vm_us: c.info[14], psb_len: c.psb, abi: c.abi,
    },
  };
}
