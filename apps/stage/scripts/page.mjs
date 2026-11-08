// Shared by render.mjs, frames.mjs and sheet.mjs: bundle the template into one
// page, start the pinned headless browser, and draw frames of it.
// Everything written lands in the folder the caller names (the work folder).
import fs from 'node:fs';
import path from 'node:path';
import {pathToFileURL} from 'node:url';
import {build} from 'esbuild';
import puppeteer from 'puppeteer-core';

/** The pinned render browser fetched by install.sh into <stage>/browser (or STAGE_BROWSER). */
export function findBrowser(stage) {
  if (process.env.STAGE_BROWSER) return process.env.STAGE_BROWSER;
  const root = path.join(stage, 'browser');
  if (!fs.existsSync(root)) return null;
  const stack = [root];
  while (stack.length) {
    const d = stack.pop();
    for (const e of fs.readdirSync(d, {withFileTypes: true})) {
      const p = path.join(d, e.name);
      if (e.isDirectory()) stack.push(p);
      else if (['chrome-headless-shell', 'chrome-headless-shell.exe'].includes(e.name)) return p;
    }
  }
  return null;
}

const HTML = `<!doctype html><meta charset="utf-8"><style>
@font-face{font-family:Fredoka;src:url(Fredoka.ttf);font-weight:300 700}
*{box-sizing:border-box}
html,body{margin:0;background:#000}#root{position:relative;width:1920px;height:1080px;overflow:hidden}
</style><div id="root"></div><script src="app.js"></script>`;

/** Bundle src/entry.tsx plus the font into pageDir (the caller removes it when done). */
export async function buildPage(stage, pageDir) {
  fs.rmSync(pageDir, {recursive: true, force: true});
  fs.mkdirSync(pageDir, {recursive: true});
  await build({
    entryPoints: [path.join(stage, 'src', 'entry.tsx')], outfile: path.join(pageDir, 'app.js'), bundle: true, format: 'iife',
    platform: 'browser', target: 'chrome120', jsx: 'automatic', define: {'process.env.NODE_ENV': '"production"'}, logLevel: 'error',
  });
  fs.copyFileSync(path.join(stage, 'fonts', 'Fredoka.ttf'), path.join(pageDir, 'Fredoka.ttf'));
  fs.writeFileSync(path.join(pageDir, 'index.html'), HTML);
  return pageDir;
}

/** Start the browser; profile and temp files go under scratchDir. */
export async function launch(executablePath, scratchDir) {
  const profile = path.join(scratchDir, 'chrome-profile');
  fs.mkdirSync(profile, {recursive: true});
  // no-sandbox: the page is our own bundle with no network, and the workshop already seals the process (bwrap)
  return puppeteer.launch({
    executablePath, headless: 'shell', pipe: true, userDataDir: profile, protocolTimeout: 120000,
    args: ['--no-sandbox', '--disable-dev-shm-usage', '--disable-gpu', '--force-color-profile=srgb', '--font-render-hinting=none',
      '--disable-crash-reporter', '--no-first-run', '--hide-scrollbars'],
    env: {...process.env, HOME: process.env.HOME || scratchDir, XDG_CONFIG_HOME: path.join(scratchDir, 'xdg-config'), XDG_CACHE_HOME: path.join(scratchDir, 'xdg-cache')},
  });
}

/** A page showing `show` ({kind, props, fps, durationInFrames}), fonts loaded, ready for draw(). */
export async function openStage(browser, pageDir, show) {
  const page = await browser.newPage();
  await page.setViewport({width: 1920, height: 1080, deviceScaleFactor: 1});
  page.on('pageerror', (e) => console.error('stage page error:', e.message));
  await page.goto(pathToFileURL(path.resolve(pageDir, 'index.html')).href);
  await page.evaluate(async () => { await document.fonts.load('600 40px Fredoka'); await document.fonts.load('700 40px Fredoka'); });
  await page.evaluate((s) => window.stage.show(s), show);
  return page;
}

export async function drawFrame(page, frame, shot = {type: 'png'}) {
  await page.evaluate((f) => window.stage.draw(f), frame);
  return page.screenshot({...shot, captureBeyondViewport: false, optimizeForSpeed: true});
}

/**
 * Film `frames` (a list of frame numbers) with `workers` pages and hand each image to
 * sink(image, index) in list order, never more than a few frames ahead of the sink.
 */
export async function filmFrames({browser, pageDir, show, frames, workers, shot, sink, onProgress}) {
  const pages = await Promise.all(Array.from({length: workers}, () => openStage(browser, pageDir, show)));
  const done = new Map();
  let next = 0, written = 0;
  const ahead = workers * 3;
  let waiting = [];
  const poke = () => { const w = waiting; waiting = []; w.forEach((r) => r()); };
  const pump = async () => {
    while (done.has(written)) { const img = done.get(written); done.delete(written); await sink(img, written); written++; onProgress?.(written / frames.length); }
  };
  await Promise.all(pages.map(async (page) => {
    for (;;) {
      while (next - written >= ahead) await new Promise((r) => waiting.push(r));
      const i = next++;
      if (i >= frames.length) return;
      done.set(i, await drawFrame(page, frames[i], shot));
      await pump(); poke();
    }
  }));
  await pump();
  await Promise.all(pages.map((p) => p.close()));
}
