// Pictures of every view of the app with an invented state, for a redesign
// brief. It serves the showcase (`showcase.html`, `src/showcase/`) with Vite,
// opens each scenario in headless Chromium at the window's size, at 2x, in the
// light and the dark theme, and writes one PNG per scenario and theme plus
// `manifest.json` to the directory given with `--out`.
//
//   node scripts/capture-showcase.mjs --out <dir> [--only id,id] [--theme light|dark]
//
// Nothing real is shown: the daemon is `mockIPC` and every date, name and text
// comes from `src/showcase/data`. It needs no Mac, no hub and no Orbit running.
import { chromium } from "@playwright/test";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";

const desktopDirectory = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const MAIN_WINDOW = { width: 1440, height: 900 };
const PANEL_WINDOW = { width: 340, height: 120 };
const THEMES = { light: "claro", dark: "oscuro" };
// The app reads `navigator.userAgent` to leave room for the Mac's window
// buttons; the webview of Orbit.app is WebKit on macOS.
const MAC_WEBKIT =
  "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko)";
// Bounds a hang only; each wait ends on its own event first.
const WAIT_MS = 20_000;

function options(argv) {
  const value = (name) => {
    const index = argv.indexOf(name);
    return index >= 0 ? argv[index + 1] : undefined;
  };
  const out = value("--out");
  if (!out) throw new Error("Falta --out <directorio> para las capturas");
  const only = value("--only")?.split(",").filter(Boolean) ?? null;
  const theme = value("--theme");
  return { out: path.resolve(out), only, themes: theme ? [theme] : Object.keys(THEMES) };
}

async function serve() {
  const server = await createServer({
    root: desktopDirectory,
    logLevel: "warn",
    server: { host: "127.0.0.1", port: 1520, strictPort: false },
  });
  await server.listen();
  const url = server.resolvedUrls?.local[0];
  if (!url) throw new Error("Vite no dio una dirección local");
  return { server, url: new URL("showcase.html", url).href };
}

/** The scenarios as data: what the capture needs, without their functions. */
async function readScenarios(browser, url) {
  const page = await browser.newPage();
  await page.goto(url);
  await page.waitForFunction(() => window.__showcase !== undefined, null, { timeout: WAIT_MS });
  const read = await page.evaluate(() => ({
    now: window.__showcase.now,
    scenarios: window.__showcase.scenarios.map((scenario) => ({
      id: scenario.id,
      view: scenario.view,
      state: scenario.state,
      description: scenario.description,
      panel: scenario.panel === true,
      size: scenario.size ?? null,
      ready: scenario.ready,
      steps: scenario.steps ?? [],
    })),
  }));
  await page.close();
  return read;
}

/** The mouse, by points from the top left of what `selector` finds; `release`
 *  false leaves the button down, for a drag in progress. */
async function mouse(page, selector, [x, y], by = null, release = true) {
  const box = await page.locator(selector).first().boundingBox();
  if (!box) throw new Error(`${selector} no se ve`);
  await page.mouse.move(box.x + x, box.y + y);
  await page.mouse.down();
  if (by) await page.mouse.move(box.x + x + by[0], box.y + y + by[1], { steps: 8 });
  if (release) await page.mouse.up();
}

async function step(page, action, now) {
  if ("mouse" in action) await mouse(page, action.mouse, action.at);
  else if ("drag" in action) await mouse(page, action.drag, [20, 12], action.by);
  else if ("hold" in action) await mouse(page, action.hold, action.at, action.by, false);
  else if ("select" in action)
    await page
      .locator(action.select)
      .first()
      .evaluate((element) => {
        const range = document.createRange();
        range.selectNodeContents(element);
        window.getSelection()?.removeAllRanges();
        window.getSelection()?.addRange(range);
        element.dispatchEvent(new window.MouseEvent("mouseup", { bubbles: true }));
      });
  else if ("advance" in action) await page.clock.setFixedTime(now + action.advance);
  else if ("click" in action) await page.locator(action.click).first().click();
  else if ("hover" in action) await page.locator(action.hover).first().hover();
  else if ("press" in action) await page.keyboard.press(action.press);
  else if ("fill" in action) await page.locator(action.fill).first().fill(action.text);
  else if ("wait" in action) await page.locator(action.wait).first().waitFor();
  else if ("scroll" in action)
    await page
      .locator(action.scroll)
      .first()
      .evaluate((element, end) => {
        // Only the box that scrolls moves, so the view's own header stays put;
        // the element ends a little below its top.
        const scrolls = (box) =>
          /(auto|scroll)/.test(getComputedStyle(box).overflowY) &&
          box.scrollHeight > box.clientHeight;
        let box = element.parentElement;
        while (box && !scrolls(box)) box = box.parentElement;
        if (!box) return;
        const shown = element.getBoundingClientRect();
        const frame = box.getBoundingClientRect();
        box.scrollTop += end ? shown.bottom - frame.bottom + 8 : shown.top - frame.top - 24;
      }, action.align === "end");
}

/** Waits for the scenario's frames, the fonts, its view and any lazy module. */
async function settle(page, scenario, now) {
  await page.waitForFunction(() => window.__showcase?.settled === true, null, { timeout: WAIT_MS });
  await page.evaluate(() => document.fonts.ready);
  await page.locator(scenario.ready).first().waitFor({ timeout: WAIT_MS });
  for (const action of scenario.steps) await step(page, action, now);
  await page.waitForLoadState("networkidle");
  await page.evaluate(() => document.fonts.ready);
  // Two frames: React's last commit and the paint after it.
  await page.evaluate(
    () => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done))),
  );
}

async function capture(browser, url, now, scenario, theme, out) {
  const size = scenario.size ?? (scenario.panel ? PANEL_WINDOW : MAIN_WINDOW);
  const context = await browser.newContext({
    viewport: size,
    deviceScaleFactor: 2,
    colorScheme: theme,
    locale: "es-ES",
    timezoneId: "Europe/Madrid",
    userAgent: MAC_WEBKIT,
    // Still pictures: a notice that fades out is drawn as it first shows.
    reducedMotion: "reduce",
  });
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.clock.setFixedTime(now);
  const file = `${scenario.id}-${THEMES[theme]}.png`;
  try {
    await page.goto(`${url}?s=${encodeURIComponent(scenario.id)}&tema=${theme}`);
    await settle(page, scenario, now);
    await page.screenshot({ path: path.join(out, file), animations: "disabled", caret: "hide" });
  } finally {
    await context.close();
  }
  if (errors.length > 0) throw new Error(`errores en la página: ${errors.join(" | ")}`);
  return { file, size };
}

/** With `--only`, the scenarios taken replace theirs in the manifest already there. */
async function writeManifest(out, taken, partial) {
  const file = path.join(out, "manifest.json");
  let manifest = taken;
  if (partial) {
    const known = await readFile(file, "utf8").then(JSON.parse, () => []);
    const ids = new Set(taken.map((entry) => entry.id));
    manifest = [...known.filter((entry) => !ids.has(entry.id)), ...taken];
  }
  await writeFile(file, `${JSON.stringify(manifest, null, 2)}\n`);
}

async function main() {
  const { out, only, themes } = options(process.argv.slice(2));
  await mkdir(out, { recursive: true });
  const { server, url } = await serve();
  // Chromium draws date fields in its own interface language, not the page's;
  // on Linux it takes that language from the environment.
  const browser = await chromium.launch({
    channel: "chromium",
    headless: true,
    args: ["--lang=es-ES"],
    env: { ...process.env, LANGUAGE: "es_ES", LANG: "es_ES.UTF-8" },
  });
  const manifest = [];
  const failures = [];
  try {
    const { now, scenarios } = await readScenarios(browser, url);
    for (const scenario of scenarios) {
      if (only && !only.includes(scenario.id)) continue;
      const files = {};
      let size = null;
      for (const theme of themes) {
        try {
          const taken = await capture(browser, url, now, scenario, theme, out);
          files[theme] = taken.file;
          size = taken.size;
          console.log(`ok   ${scenario.id} (${THEMES[theme]})`);
        } catch (error) {
          failures.push(`${scenario.id} (${THEMES[theme]}): ${error.message}`);
          console.log(`fallo ${scenario.id} (${THEMES[theme]}): ${error.message}`);
        }
      }
      const { id, view, state, description, panel } = scenario;
      manifest.push({
        id,
        view,
        state,
        description,
        window: panel ? "panel" : "main",
        size,
        files,
      });
    }
  } finally {
    await browser.close();
    await server.close();
  }
  await writeManifest(out, manifest, only !== null);
  console.log(`${manifest.length} escenarios en ${out}; ${failures.length} fallos`);
  if (failures.length > 0) process.exitCode = 1;
}

await main();
