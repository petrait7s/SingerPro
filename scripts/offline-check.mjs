// Fails if the production bundle references any remote resource (CDN, remote font, API).
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";

const ALLOWED = [
  /^https?:\/\/www\.w3\.org\//,
  /^https?:\/\/schema\.tauri\.app\//,
  /^http:\/\/ipc\.localhost/,
  /^https?:\/\/localhost/,
  /^https:\/\/react\.dev\/errors/,
  /^https:\/\/reactjs\.org/,
  /^https?:\/\/github\.com\/(facebook|reactjs)\//,
];
const files = [];
(function walk(d) {
  for (const n of readdirSync(d)) {
    const p = join(d, n);
    statSync(p).isDirectory() ? walk(p) : /\.(js|css|html)$/.test(n) && files.push(p);
  }
})("dist");

const bad = [];
for (const f of files) {
  const text = readFileSync(f, "utf8");
  for (const m of text.matchAll(/https?:\/\/[^\s"'`)<>\\]+/g)) {
    if (!ALLOWED.some((r) => r.test(m[0]))) bad.push(`${f}: ${m[0]}`);
  }
}
if (bad.length) {
  console.error("External URLs found in bundle:\n" + [...new Set(bad)].join("\n"));
  process.exit(1);
}
console.log(`offline-check OK: ${files.length} files, no remote resources`);
