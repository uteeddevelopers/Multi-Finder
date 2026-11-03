#!/usr/bin/env node
// Multi-Finder for Windows — string tables.
//
// Converts the macOS string catalog (../Packaging/Localizable.xcstrings; keys are the Korean source
// strings) plus src/i18n/extra.json (Windows-only strings) into
//   src/i18n/{ko,en,ja,zh-Hans}.json   { "<korean key>": "text" | { "one": …, "other": … } }
//   src/i18n/keys.ts                   `MsgKey` union type, so every t("…") call is checked by tsc.
//
// Format conversion
//   %@ %lld %d …            → {0} {1} … (in order of appearance)
//   %1$@ %2$lld …           → {0} {1} … (explicit position − 1)
//   %%                       → %
//   top-level plural          → { "one": "…", "other": "…" }  (chosen by the first numeric argument)
//   %2$#@name@ substitution  → {1, plural, one {# file} other {# files}}  (inline, ICU-like; `#` = the number)
//
// extra.json: { "<korean key>": { "en": …, "ja": …, "zh-Hans": …, "ko"?: … } } — values are strings
// (with {0} placeholders / inline plurals) or plural objects. A key that also exists in the catalog
// overrides that key's translations for the languages it lists (used where the Korean wording is the
// same on Windows but the other languages differ, e.g. Trash → Recycle Bin, Quick Look → Preview).
// Keys starting with "$" are comments.

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, "..");
const catalogPath = process.env.MF_XCSTRINGS ?? path.resolve(root, "../Packaging/Localizable.xcstrings");
const outDir = path.join(root, "src", "i18n");
const extraPath = path.join(outDir, "extra.json");
const LANGS = ["ko", "en", "ja", "zh-Hans"];

const SPEC = /%(?:(\d+)\$)?(#@([A-Za-z0-9_]+)@|lld|lli|llu|ld|li|lu|d|i|u|@|lf|f|\.\d+f|%)/g;

/** Converts one Apple format string into the `{n}` / inline-plural syntax used by src/i18n/index.ts. */
function convert(str, substitutions) {
  let seq = 0;
  return str.replace(SPEC, (_m, pos, spec, subName) => {
    if (spec === "%") return "%";
    const idx = pos ? Number(pos) - 1 : seq++;
    if (subName) {
      const sub = substitutions?.[subName];
      if (!sub) throw new Error(`unknown substitution %#@${subName}@ in ${JSON.stringify(str)}`);
      const argIdx = pos ? Number(pos) - 1 : sub.argNum ? sub.argNum - 1 : idx;
      const plural = sub.variations?.plural;
      if (!plural) throw new Error(`substitution ${subName} has no plural variations`);
      const branches = Object.entries(plural).map(([cat, v]) => {
        const inner = v.stringUnit.value.replace(/%arg/g, "#").replace(/%(?:\d+\$)?(?:lld|ld|d|@)/g, "#");
        return `${cat} {${inner}}`;
      });
      return `{${argIdx}, plural, ${branches.join(" ")}}`;
    }
    return `{${idx}}`;
  });
}

function localized(loc) {
  if (!loc) return undefined;
  if (loc.stringUnit) return convert(loc.stringUnit.value, loc.substitutions);
  if (loc.variations?.plural) {
    const out = {};
    for (const [cat, v] of Object.entries(loc.variations.plural)) out[cat] = convert(v.stringUnit.value, loc.substitutions);
    return out;
  }
  return undefined;
}

const catalog = JSON.parse(fs.readFileSync(catalogPath, "utf8"));
const tables = Object.fromEntries(LANGS.map((l) => [l, {}]));
const missing = Object.fromEntries(LANGS.map((l) => [l, []]));
let catalogCount = 0;

for (const [rawKey, entry] of Object.entries(catalog.strings)) {
  if (rawKey === "") continue;
  const key = convert(rawKey);
  catalogCount++;
  const locs = entry.localizations ?? {};
  for (const lang of LANGS) {
    let value = localized(locs[lang]);
    if (value === undefined) {
      // Format-only strings ("%@ · %@", "--") have no translations: the key itself is the text.
      // Korean is the source language, so its key is its text.
      if (lang !== "ko" && Object.keys(locs).length > 0) missing[lang].push(key);
      value = key;
    }
    tables[lang][key] = value;
  }
}

const extra = fs.existsSync(extraPath) ? JSON.parse(fs.readFileSync(extraPath, "utf8")) : {};
const overridden = [];
let extraCount = 0;
for (const [key, entry] of Object.entries(extra)) {
  if (key.startsWith("$")) continue;
  const exists = key in tables.ko;
  if (exists) overridden.push(key);
  else extraCount++;
  for (const lang of LANGS) {
    const v = entry[lang];
    if (v !== undefined) tables[lang][key] = v;
    else if (lang === "ko") tables.ko[key] = tables.ko[key] ?? key;
    else if (!exists) {
      missing[lang].push(key);
      tables[lang][key] = key;
    }
  }
}

// Sanity check: every {n} a translation uses must exist in the Korean key.
const placeholders = (s) => new Set([...String(s).matchAll(/\{(\d+)(?:\}|,)/g)].map((m) => m[1]));
const problems = [];
for (const lang of LANGS) {
  for (const [key, value] of Object.entries(tables[lang])) {
    const allowed = placeholders(key);
    const texts = typeof value === "string" ? [value] : Object.values(value);
    for (const text of texts) for (const p of placeholders(text)) if (!allowed.has(p)) problems.push(`${lang}: {${p}} not in key ${JSON.stringify(key)}`);
  }
}

fs.mkdirSync(outDir, { recursive: true });
const sortKeys = (obj) => Object.fromEntries(Object.keys(obj).sort().map((k) => [k, obj[k]]));
for (const lang of LANGS) {
  fs.writeFileSync(path.join(outDir, `${lang}.json`), JSON.stringify(sortKeys(tables[lang]), null, 1) + "\n");
}
const keys = Object.keys(tables.ko).sort();
fs.writeFileSync(
  path.join(outDir, "keys.ts"),
  "// Generated by tools/gen-i18n.mjs — do not edit. Every user-visible string key (Korean source text).\n" +
    "export type MsgKey =\n" +
    keys.map((k) => `  | ${JSON.stringify(k)}`).join("\n") +
    ";\n",
);

console.log(`i18n: ${catalogCount} catalog keys + ${extraCount} Windows keys (extra.json), ${overridden.length} catalog keys overridden for Windows wording → ${keys.length} keys`);
for (const lang of LANGS) if (missing[lang].length) console.log(`  ${lang}: ${missing[lang].length} without translation (text = key): ${missing[lang].slice(0, 8).map((k) => JSON.stringify(k)).join(", ")}${missing[lang].length > 8 ? " …" : ""}`);
if (process.argv.includes("--verbose")) for (const k of overridden) console.log(`  override: ${JSON.stringify(k)}`);
if (problems.length) {
  console.error(problems.join("\n"));
  process.exit(1);
}
