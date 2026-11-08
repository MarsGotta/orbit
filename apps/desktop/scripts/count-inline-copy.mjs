// The UI gate imports the same AST scanner, so this inventory cannot drift
// from enforcement. Exceptions document machine tokens and separate ownership.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { inlineCopy } from "./inline-copy-scan.mjs";

const root = fileURLToPath(new URL("../src/", import.meta.url));
const packageRoot = fileURLToPath(new URL("../../../packages/orbit-ui/", import.meta.url));
const counts = new Map();
function walk(base, dir) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const file = path.join(dir, entry.name);
    if (entry.isDirectory() && !entry.name.startsWith("__")) walk(base, file);
    else if (/\.tsx?$/.test(entry.name) && !/(\.test\.|^test[-.])/.test(entry.name)) {
      const relative = path.relative(base, file);
      const candidates = inlineCopy(relative, fs.readFileSync(file, "utf8"));
      const group = relative.startsWith("features/")
        ? relative.split(path.sep)[1]
        : relative.split(path.sep)[0];
      counts.set(group, (counts.get(group) ?? 0) + candidates.length);
      if (process.argv.includes("--check"))
        for (const candidate of candidates)
          console.error(`${relative}:${candidate.line}: ${candidate.text}`);
    }
  }
}
for (const base of [root, packageRoot])
  for (const folder of ["features", "app", "design", "state"]) {
    const dir = path.join(base, folder);
    if (fs.existsSync(dir)) walk(base, dir);
  }
console.log("| UI folder | Inline copy candidates |\n|---|---:|");
for (const [feature, count] of [...counts].sort()) console.log(`| ${feature} | ${count} |`);
const total = [...counts.values()].reduce((a, b) => a + b, 0);
console.log(`| **Total** | **${total}** |`);
if (process.argv.includes("--check") && total > 0) process.exitCode = 1;
