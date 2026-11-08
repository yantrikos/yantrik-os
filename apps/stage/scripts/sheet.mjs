// QA: node scripts/sheet.mjs <mode> <rig> <out.png> [frame]  (cast|expressions|visemes|poses)
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {buildPage, findBrowser, launch, openStage, drawFrame} from './page.mjs';

const stage = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const [mode, rig, out, frame] = process.argv.slice(2);
const exe = findBrowser(stage);
if (!exe) { console.error('sheet: the render browser is not installed (install.sh).'); process.exit(1); }
const scratch = fs.mkdtempSync(path.join(process.env.TMPDIR || os.tmpdir(), 'stage-sheet-'));
process.on('exit', () => fs.rmSync(scratch, {recursive: true, force: true}));
const pageDir = await buildPage(stage, path.join(scratch, 'page'));
const browser = await launch(exe, scratch);
try {
  const page = await openStage(browser, pageDir, {kind: 'sheet', props: {mode, rig}, fps: 30, durationInFrames: 90});
  fs.writeFileSync(out, await drawFrame(page, Number(frame || 20)));
} finally { await browser.close(); }
