#!/usr/bin/env node
// Render step: bundle the template into one page, draw every frame in the pinned
// headless browser (puppeteer-core), pipe the pictures to ffmpeg with the mixed
// sound, write out/<title>.mp4 and out/contact.png (a 3x3 grid of frames).
// Skips work when nothing changed.
// Usage: render.mjs <work-dir> <out-dir>
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import crypto from 'node:crypto';
import {spawn, spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import {buildPage, findBrowser, launch, filmFrames} from './page.mjs';
import {buildAudio} from './audio.mjs';

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
for (const d of ['src', 'fonts', 'scripts']) walk(path.join(stage, d));
h.update(JSON.stringify(episode)).update(JSON.stringify(timeline));
const stamp = h.digest('hex');
const stampFile = path.join(out, '.stage-hash');
const fresh = fs.existsSync(mp4) && fs.existsSync(stampFile) && fs.readFileSync(stampFile, 'utf8') === stamp;
const dur = timeline.totalFrames / timeline.fps;
// Contact sheet: nine frames across the episode, tiled 3x3.
const makeContact = () => {
  const r = spawnSync('ffmpeg', ['-y', '-v', 'error', '-ss', String(dur / 20), '-i', mp4, '-vf',
    `fps=10/${dur},scale=640:360,tile=3x3:padding=6:color=0x120f24`, '-frames:v', '1', contact], {encoding: 'utf8'});
  if (r.status !== 0) die(`could not make the contact sheet: ${r.stderr}`);
};
if (fresh) {
  if (!fs.existsSync(contact)) makeContact();
  fs.writeFileSync(path.join(out, 'video'), mp4 + '\n');
  fs.appendFileSync(path.join(out, 'progress'), 'render: done (up to date)\n');
  console.log(`render: up to date, nothing to film (${path.basename(mp4)})`);
  process.exit(0);
}

const browserExecutable = findBrowser(stage);
if (!browserExecutable) die('the render browser is not installed. Run install.sh once (it fetches the pinned chrome-headless-shell).');
fs.mkdirSync(out, {recursive: true});

// Everything the browser and ffmpeg write goes under the work folder, removed when the film is done.
const pageDir = path.join(work, 'page'), tmpDir = path.join(work, 'tmp');
fs.mkdirSync(tmpDir, {recursive: true});
process.env.TMPDIR = tmpDir;
process.on('exit', () => { for (const d of [pageDir, tmpDir]) fs.rmSync(d, {recursive: true, force: true}); });
const t0 = Date.now();
// out/progress: one line per step, newest last, so a mind can read the end of it cheaply
const progress = path.join(out, 'progress');
const say = (line) => fs.appendFileSync(progress, line + '\n');
say('render: 0%');
await buildPage(stage, pageDir);
const workers = Number(process.env.STAGE_CONCURRENCY) || Math.max(1, Math.min(3, os.cpus().length - 1));
const audio = buildAudio(timeline, path.join(work, 'public'));
const total = timeline.totalFrames;
const ff = spawn('ffmpeg', ['-y', '-v', 'error', '-f', 'image2pipe', '-framerate', String(timeline.fps), '-c:v', 'mjpeg', '-i', 'pipe:0',
  ...audio.inputs.flatMap((f) => ['-i', f]),
  ...(audio.map ? ['-filter_complex', audio.graph] : []), '-map', '0:v', ...(audio.map ? ['-map', audio.map, '-c:a', 'aac', '-b:a', '192k', '-ac', '2'] : []),
  '-c:v', 'libx264', '-crf', '19', '-pix_fmt', 'yuv420p', '-r', String(timeline.fps), '-movflags', '+faststart', '-frames:v', String(total), mp4],
  {stdio: ['pipe', 'inherit', 'pipe']});
let ffErr = '';
ff.stderr.on('data', (d) => { ffErr += d; });
let filming = true;
const ffDone = new Promise((res) => ff.on('close', (code) => { if (filming && code !== 0) die('ffmpeg stopped early: ' + ffErr.split('\n').slice(-4).join(' ')); res(code); }));
ff.stdin.on('error', () => {});   // ffmpeg died early: its exit code says why
const browser = await launch(browserExecutable, tmpDir);
let lastPct = -1;
try {
  await filmFrames({
    browser, pageDir, workers, frames: Array.from({length: total}, (_, i) => i),
    show: {kind: 'episode', props: {episode, timeline}, fps: timeline.fps, durationInFrames: total},
    shot: {type: 'jpeg', quality: 88},
    sink: (img) => ff.stdin.write(img) || new Promise((r) => ff.stdin.once('drain', r)),
    onProgress: (p) => {
      const pct = Math.floor(p * 20) * 5;
      if (pct !== lastPct) { lastPct = pct; say(`render: ${pct}%`); if (pct % 10 === 0) console.log(`render: ${pct}% (${((Date.now() - t0) / 1000).toFixed(0)}s)`); }
    },
  });
} finally { filming = false; await browser.close(); }
ff.stdin.end();
if ((await ffDone) !== 0) die(`ffmpeg failed: ${ffErr.split('\n').slice(-4).join(' ')}`);

makeContact();
fs.writeFileSync(stampFile, stamp);
fs.writeFileSync(path.join(out, 'video'), mp4 + '\n');
say('render: done');
const secs = ((Date.now() - t0) / 1000).toFixed(0);
const mb = (fs.statSync(mp4).size / 1048576).toFixed(1);
console.log(`render: done in ${secs}s -> ${mp4} (${dur.toFixed(1)}s video, ${mb} MB), ${contact}`);
