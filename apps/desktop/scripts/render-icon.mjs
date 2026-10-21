import { chromium } from "@playwright/test";
import { spawn } from "node:child_process";
import { fileURLToPath, pathToFileURL } from "node:url";
import path from "node:path";

const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
const desktopDirectory = path.resolve(scriptDirectory, "..");
const source = path.join(desktopDirectory, "src-tauri", "icons", "source.svg");
const output = path.join(desktopDirectory, "src-tauri", "icons", "icon-1024.png");

// The Chromium channel uses the full browser in headless mode. Playwright also
// offers a smaller headless-shell download, but requiring both would make icon
// generation depend on an artifact the desktop test suite does not otherwise use.
const browser = await chromium.launch({ channel: "chromium", headless: true });
try {
  const page = await browser.newPage({
    deviceScaleFactor: 1,
    viewport: { width: 1024, height: 1024 },
  });
  await page.goto(pathToFileURL(source).href);
  await page.locator("svg").screenshot({ omitBackground: true, path: output });
} finally {
  await browser.close();
}

const iconCommand = spawn("pnpm", ["tauri", "icon", output], {
  cwd: desktopDirectory,
  stdio: "inherit",
});
const exitCode = await new Promise((resolve, reject) => {
  iconCommand.once("error", reject);
  iconCommand.once("exit", resolve);
});
if (exitCode !== 0) {
  throw new Error(`pnpm tauri icon terminó con código ${exitCode}`);
}
