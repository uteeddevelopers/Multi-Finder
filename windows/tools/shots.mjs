#!/usr/bin/env node
// Screenshots of every screen with the mock backend: ko + en, light + dark, 1280×800 → shots/.
// Usage: npm run build && npm run shots   (optional: --lang=ko,en,ja,zh --theme=light,dark --only=compare,leftovers)
// Uses Playwright from node_modules if present, otherwise a globally installed one (NODE_PATH).
import http from "node:http";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, "..");
const dist = path.join(root, "dist");
const outDir = path.join(root, "shots");

const arg = (name, fallback) => {
  const a = process.argv.find((x) => x.startsWith(`--${name}=`));
  return a ? a.slice(name.length + 3).split(",").filter(Boolean) : fallback;
};
const LANGS = arg("lang", ["ko", "en"]);
const THEMES = arg("theme", ["light", "dark"]);
const ONLY = arg("only", null);

if (!fs.existsSync(path.join(dist, "index.html"))) {
  console.error("dist/ not found — run `npm run build` first.");
  process.exit(1);
}

const require = createRequire(import.meta.url);
let playwright;
for (const name of ["playwright", "playwright-core", "@playwright/test"]) {
  try {
    playwright = require(name);
    break;
  } catch {
    /* try the next one */
  }
}
if (!playwright) {
  console.error("Playwright is not installed (npm i -D playwright, or put a global install on NODE_PATH).");
  process.exit(1);
}

const TYPES = { ".html": "text/html; charset=utf-8", ".js": "text/javascript", ".css": "text/css", ".svg": "image/svg+xml", ".png": "image/png", ".json": "application/json", ".woff2": "font/woff2", ".ico": "image/x-icon" };
const server = http.createServer((req, res) => {
  let p = decodeURIComponent(new URL(req.url ?? "/", "http://localhost").pathname);
  if (p === "/") p = "/index.html";
  const file = path.normalize(path.join(dist, p));
  if (!file.startsWith(dist) || !fs.existsSync(file) || fs.statSync(file).isDirectory()) {
    res.writeHead(404);
    res.end();
    return;
  }
  res.writeHead(200, { "content-type": TYPES[path.extname(file)] ?? "application/octet-stream" });
  fs.createReadStream(file).pipe(res);
});
await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
const base = `http://127.0.0.1:${server.address().port}/`;

const DOWNLOADS = "C:\\Users\\Alex\\Downloads";
const DESKTOP = "C:\\Users\\Alex\\Desktop";

const settle = (page, ms = 450) => page.waitForTimeout(ms);
const row = (page, text) => page.locator('[role="option"]', { hasText: text }).first();

/** Each scenario: URL params, optional expanded folders (localStorage), and steps before the shot. */
const SCENARIOS = [
  {
    name: "all-folders",
    expanded: [DOWNLOADS],
    run: async (page) => {
      await page.waitForSelector(".tree-row");
      await settle(page, 700);
    },
  },
  {
    name: "search",
    params: { search: "invoice" },
    run: async (page) => {
      await page.waitForSelector(".search-row");
      await row(page, "invoice-northwind-2026-10.pdf").click();
      await settle(page);
    },
  },
  {
    name: "preview",
    expanded: [DOWNLOADS],
    run: async (page) => {
      await page.waitForSelector(".tree-row");
      await row(page, "mountain-lake.jpg").click();
      await page.keyboard.press("Space");
      await page.waitForSelector(".pv-media");
      await settle(page);
    },
  },
  {
    name: "preview-text",
    expanded: [DESKTOP],
    run: async (page) => {
      await page.waitForSelector(".tree-row");
      await row(page, "meeting-notes.md").click();
      await page.keyboard.press("Space");
      await page.waitForSelector(".pv-text");
      await settle(page);
    },
  },
  {
    name: "duplicates",
    params: { screen: "duplicates", run: "1" },
    run: async (page) => {
      await page.waitForSelector(".file-row", { timeout: 15000 });
      await page.locator(".tree-group-row").first().click();
      await page.locator(".file-row").nth(1).click();
      await settle(page);
    },
  },
  {
    name: "duplicates-intro",
    params: { screen: "duplicates" },
    run: async (page) => {
      await page.waitForSelector(".empty-state");
      await settle(page, 200);
    },
  },
  {
    name: "compare",
    params: { screen: "compare", run: "1" },
    run: async (page) => {
      await page.waitForSelector(".cmp-row", { timeout: 15000 });
      await page.locator(".cmp-row").nth(9).click();
      await settle(page);
    },
  },
  {
    name: "categories-model",
    params: { screen: "categories" },
    run: async (page) => {
      await page.waitForSelector(".model-card");
      await settle(page, 200);
    },
  },
  {
    name: "categories-downloading",
    params: { screen: "categories", model: "downloading" },
    run: async (page) => {
      await page.waitForSelector(".mc-progress");
      await settle(page, 200);
    },
  },
  {
    name: "categories-error",
    params: { screen: "categories", model: "error" },
    run: async (page) => {
      await page.waitForSelector(".mc-error");
      await settle(page, 200);
    },
  },
  {
    name: "categories",
    params: { screen: "categories", classified: "1" },
    run: async (page) => {
      await page.waitForSelector(".cat-row");
      await page.locator(".cat-row .cat-file").nth(2).click();
      await settle(page);
    },
  },
  {
    name: "leftovers",
    params: { screen: "leftovers", run: "1" },
    run: async (page) => {
      await page.waitForSelector(".lo-app", { timeout: 15000 });
      await page.locator(".lo-app .chev").first().click();
      await page.locator(".lo-app").first().click();
      await settle(page);
    },
  },
  {
    name: "context-menu",
    expanded: [DOWNLOADS],
    run: async (page) => {
      await page.waitForSelector(".tree-row");
      await row(page, "Invoices").click({ button: "right" });
      await page.waitForSelector('[role="menu"]');
      await settle(page, 300);
    },
  },
  {
    name: "filter-menu",
    expanded: [DOWNLOADS],
    run: async (page) => {
      await page.waitForSelector(".tree-row");
      await page.locator(".tb-actions button", { hasText: /Filter|필터/ }).click();
      await page.locator('[role="menuitem"]').first().click();
      await page.locator('[role="menuitemradio"]').nth(6).click();
      await page.locator(".tb-actions button", { hasText: /Filter|필터/ }).click();
      await page.locator('[role="menuitem"]').nth(1).hover();
      await page.waitForSelector('[role="menuitemradio"]');
      await settle(page, 400);
    },
  },
  {
    name: "bucket-editor",
    params: { screen: "categories", classified: "1" },
    run: async (page) => {
      await page.waitForSelector(".cat-row");
      await page.locator(".tb-actions button", { hasText: /Edit Categories|묶음 편집/ }).click();
      await page.waitForSelector(".be-row");
      await settle(page, 300);
    },
  },
  {
    name: "confirm-trash",
    expanded: [DOWNLOADS],
    run: async (page) => {
      await page.waitForSelector(".tree-row");
      await row(page, "Screenshot 2026-10-21 094512.png").click();
      await row(page, "Screenshot 2026-10-29 173008.png").click({ modifiers: ["Control"] });
      await row(page, "OldEditor-Setup-2.0.exe").click({ modifiers: ["Control"] });
      await page.keyboard.press("Delete");
      await page.waitForSelector('[role="alertdialog"]');
      await settle(page, 500);
    },
  },
];

fs.mkdirSync(outDir, { recursive: true });
const browser = await playwright.chromium.launch();
const written = [];
const failures = [];
try {
  for (const lang of LANGS) {
    for (const theme of THEMES) {
      for (const sc of SCENARIOS) {
        if (ONLY && !ONLY.includes(sc.name)) continue;
        const context = await browser.newContext({ viewport: { width: 1280, height: 800 }, colorScheme: theme, deviceScaleFactor: 1, locale: lang });
        if (sc.expanded) {
          await context.addInitScript((value) => {
            try {
              localStorage.setItem("mf.expanded", value);
            } catch {
              /* ignore */
            }
          }, JSON.stringify(sc.expanded));
        }
        const page = await context.newPage();
        const errors = [];
        page.on("pageerror", (e) => errors.push(e.message));
        page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
        const params = new URLSearchParams({ lang, fast: "1", ...(sc.params ?? {}) });
        const file = path.join(outDir, `${lang}-${theme}-${sc.name}.png`);
        try {
          await page.goto(`${base}?${params}`);
          await sc.run(page);
          await page.mouse.move(1270, 790); // no hover highlight in the shot
          await settle(page, 150);
          await page.screenshot({ path: file });
          written.push(path.relative(root, file));
        } catch (e) {
          failures.push(`${lang}-${theme}-${sc.name}: ${e.message.split("\n")[0]}`);
        }
        if (errors.length) failures.push(`${lang}-${theme}-${sc.name} console: ${errors.slice(0, 3).join(" | ")}`);
        await context.close();
      }
    }
  }
} finally {
  await browser.close();
  server.close();
}
console.log(`${written.length} screenshots → ${path.relative(process.cwd(), outDir) || "."}`);
if (failures.length) {
  console.error(failures.join("\n"));
  process.exitCode = 1;
}
