// Renders the real UI (backend stubbed) in Chromium and verifies bundled Bangla/English fonts load
// with no network access beyond localhost. Run: pnpm build && node scripts/font-check.mjs
import { createRequire } from "node:module";
import { execSync } from "node:child_process";
// playwright is a global/pre-installed tool, not a project dependency; resolve it from the global root.
const require = createRequire(import.meta.url);
let pw;
try {
  pw = require("playwright");
} catch {
  pw = createRequire(execSync("npm root -g").toString().trim() + "/").call
    ? require(execSync("npm root -g").toString().trim() + "/playwright")
    : null;
}
const { chromium } = pw;
import { spawn } from "node:child_process";

const PORT = 4179;
const srv = spawn("pnpm", ["exec", "vite", "preview", "--port", String(PORT), "--strictPort"], {
  stdio: "ignore",
});
await new Promise((r) => setTimeout(r, 2500));
const browser = await chromium.launch();
let failed = false;
const external = [];
try {
  const page = await browser.newPage({
    viewport: { width: 1440, height: 900 },
    deviceScaleFactor: 1,
  });
  page.on("request", (r) => {
    const u = new URL(r.url());
    if (!["localhost", "127.0.0.1"].includes(u.hostname) && u.protocol.startsWith("http"))
      external.push(r.url());
  });
  await page.addInitScript(() => {
    localStorage.setItem("sp.lang", "bn");
    window.__TAURI_INTERNALS__ = {
      transformCallback: () => 0,
      invoke: async (cmd) => {
        if (cmd === "setup_status") return { needs_setup: true, startup_notice: null };
        throw { code: "unknown", message: cmd };
      },
    };
  });
  await page.goto(`http://localhost:${PORT}`);
  await page.waitForSelector("text=সিঙ্গার প্রো-তে স্বাগতম");
  // mixed Bangla + English + ৳ + Bengali & Latin digits, to exercise shaping and fallback
  await page.evaluate(() => {
    const d = document.createElement("div");
    d.id = "probe";
    d.style.cssText =
      "position:fixed;left:20px;bottom:20px;padding:12px;background:#fff;font-size:18px;z-index:99";
    d.innerHTML =
      '<p lang="bn">যুক্তাক্ষর: ক্ষ ন্ত্র শ্রী র‍্য কিস্তি ৳১,২৫,০০০.৫০ · Total ৳1,25,000.50</p><p class="bn-editorial">সম্পাদকীয় শিরোনাম — Tiro Bangla</p><p class="metric-display">৳ 1,25,000.50 Manrope</p>';
    document.body.append(d);
  });
  await page.evaluate(() => document.fonts.ready);
  await page.waitForTimeout(500);
  const loaded = await page.evaluate(() => [
    ...new Set(
      [...document.fonts]
        .filter((f) => f.status === "loaded")
        .map((f) => f.family.replace(/"/g, "")),
    ),
  ]);
  console.log("loaded font families:", loaded.join(", "));
  for (const fam of ["Hind Siliguri", "Tiro Bangla", "Montserrat Variable", "Manrope Variable"]) {
    if (!loaded.includes(fam)) {
      console.error(`FAIL: ${fam} not loaded`);
      failed = true;
    }
  }
  const ok = await page.evaluate(() => document.fonts.check('16px "Hind Siliguri"', "ক্ষ ৳"));
  if (!ok) {
    console.error("FAIL: Hind Siliguri cannot render probe glyphs");
    failed = true;
  }
  const h1 = await page.evaluate(() => getComputedStyle(document.querySelector("h1")).fontFamily);
  if (!h1.includes("Tiro Bangla")) {
    console.error("FAIL: Bangla heading not using Tiro Bangla:", h1);
    failed = true;
  }
  await page.screenshot({ path: process.env.SHOT ?? "font-check.png" });
  if (external.length) {
    console.error("FAIL: external requests:", external);
    failed = true;
  }
} finally {
  await browser.close();
  srv.kill();
}
console.log(
  failed
    ? "font-check FAILED"
    : "font-check OK (fonts bundled, Bangla renders, no external requests)",
);
process.exit(failed ? 1 : 0);
