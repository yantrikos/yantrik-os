#!/usr/bin/env node
// Render step: bundle the Remotion template, film it offline with the
// browser that was installed with Stage, write out/<title>.mp4 and
// out/contact.png (a 3x3 grid of frames). Skips work when nothing changed.
// Usage: render.mjs <work-dir> <out-dir>
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import crypto from 'node:crypto';
import {spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import {bundle} from '@remotion/bundler';
import {renderMedia, selectComposition} from '@remotion/renderer';

const here = path.dirname(fileURLToPath(import.meta.url));
const stage = path.resolve(here, '..');
const [work, out] = process.argv.slice(2);
if (!work || !out) { console.error('usage: render.mjs <work-dir> <out-dir>'); process.exit(2); }
const die = (m) => { console.error(`stage render: ${m}`); process.exit(1); };

const episode = JSON.parse(fs.readFileSync(path.join(work, 'episode.json'), 'utf8'));
const timeline = JSON.parse(fs.readFileSync(path.join(work, 'timeline.json'), 'utf8'));
const safe = episode.title.replace(/[^A-Za-z0-9 _-]/g, '').trim().replace(/\s+/g, '-') || 'episode';
const mp4 = path.join(out, `${safe}.mp4`);
const contact = path.join(out, 'contact.png');

// Input hash: the episode, the voices and every source file of the template.
const h = crypto.createHash('sha1');
const walk = (d) => fs.readdirSync(d, {withFileTypes: true}).forEach((e) => {
  const p = path.join(d, e.name);
  e.isDirectory() ? walk(p) : h.update(e.name).update(fs.readFileSync(p));
});
walk(path.join(stage, 'src'));
h.update(JSON.stringify(episode)).update(JSON.stringify(timeline));
const stamp = h.digest('hex');
const stampFile = path.join(out, '.stage-hash');
const fresh = fs.existsSync(mp4) && fs.existsSync(stampFile) && fs.readFileSync(stampFile, 'utf8') === stamp;
const dur = timeline.totalFrames / timeline.fps;
// Contact sheet: nine frames across the episode, tiled 3x3.
const makeContact = () => {
  const r = spawnSync('ffmpeg', ['-y', '-v', 'error', '-ss', String(dur / 20), '-i', mp4, '-vf', `fps=10/${dur},scale=640:360,tile=3x3:padding=6:color=0x120f24`, '-frames:v', '1', contact], {encoding: 'utf8'});
  if (r.status !== 0) die(`could not make the contact sheet: ${r.stderr}`);
};
if (fresh) {
  if (!fs.existsSync(contact)) makeContact();
  console.log(`render: up to date, nothing to film (${path.basename(mp4)})`);
  process.exit(0);
}

// The browser Remotion fetched into node_modules/.remotion at install time.
const findBrowser = () => {
  const root = path.join(stage, 'node_modules', '.remotion');
  if (!fs.existsSync(root)) return null;
  const stack = [root];
  while (stack.length) {
    const d = stack.pop();
    for (const e of fs.readdirSync(d, {withFileTypes: true})) {
      const p = path.join(d, e.name);
      if (e.isDirectory()) stack.push(p);
      else if (e.name === 'chrome-headless-shell' || e.name === 'headless_shell') return p;
    }
  }
  return null;
};
const browserExecutable = process.env.STAGE_BROWSER || findBrowser();
if (!browserExecutable) die('the render browser is not installed. Run install.sh once (it runs "npx remotion browser ensure").');

fs.mkdirSync(path.join(work, 'public', 'fonts'), {recursive: true});
fs.copyFileSync(path.join(stage, 'fonts', 'Fredoka.ttf'), path.join(work, 'public', 'fonts', 'Fredoka.ttf'));
fs.mkdirSync(out, {recursive: true});

const t0 = Date.now();
const serveUrl = await bundle({entryPoint: path.join(stage, 'src', 'index.ts'), publicDir: path.join(work, 'public'), onProgress: () => {}, webpackOverride: (c) => ({...c, cache: false})});
const inputProps = {episode, timeline};
const composition = await selectComposition({serveUrl, id: 'Episode', inputProps, browserExecutable, logLevel: 'error'});
const concurrency = Number(process.env.STAGE_CONCURRENCY) || Math.max(1, Math.min(3, os.cpus().length - 1));
let lastPct = -1;
await renderMedia({
  composition, serveUrl, inputProps, browserExecutable, concurrency, logLevel: 'error',
  codec: 'h264', crf: 19, pixelFormat: 'yuv420p', imageFormat: 'jpeg', jpegQuality: 88, audioCodec: 'aac',
  outputLocation: mp4, overwrite: true,
  onProgress: ({progress}) => {
    const pct = Math.floor(progress * 10) * 10;
    if (pct !== lastPct) { lastPct = pct; console.log(`render: ${pct}% (${((Date.now() - t0) / 1000).toFixed(0)}s)`); }
  },
});

makeContact();
fs.writeFileSync(stampFile, stamp);
const secs = ((Date.now() - t0) / 1000).toFixed(0);
const mb = (fs.statSync(mp4).size / 1048576).toFixed(1);
console.log(`render: done in ${secs}s -> ${mp4} (${dur.toFixed(1)}s video, ${mb} MB), ${contact}`);
