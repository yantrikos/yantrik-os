#!/usr/bin/env node
// Golden frames: draw the frames saved in goldens/<name>/ again from the saved
// episode.json and timeline.json, and compare each with its golden (mean
// absolute difference per pixel, 0-255 scale, from ffmpeg's blend + signalstats).
//   golden.mjs [dir] [--limit 0.05] [--update]     exit 1 when any frame is over the limit
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const args = process.argv.slice(2);
const flag = (n) => { const i = args.indexOf(n); return i < 0 ? null : args.splice(i, 1)[0]; };
const update = flag('--update');
const li = args.indexOf('--limit');
const limit = li < 0 ? 0.05 : Number(args.splice(li, 2)[1]);
const dir = path.resolve(args[0] ?? path.join(here, '..', 'goldens', 'the-red-light'));
const frames = fs.readdirSync(dir).filter((f) => /^f\d+\.png$/.test(f)).sort();
const scratch = fs.mkdtempSync(path.join(process.env.TMPDIR || os.tmpdir(), 'stage-golden-'));
process.on('exit', () => fs.rmSync(scratch, {recursive: true, force: true}));
for (const f of ['episode.json', 'timeline.json']) fs.copyFileSync(path.join(dir, f), path.join(scratch, f));
const drawn = path.join(scratch, 'drawn');
const r = spawnSync('node', [path.join(here, 'frames.mjs'), scratch, drawn, ...frames.map((f) => String(Number(f.slice(1, -4))))], {stdio: 'inherit'});
if (r.status !== 0) process.exit(r.status ?? 1);
let bad = 0;
for (const f of frames) {
  if (update) { fs.copyFileSync(path.join(drawn, f), path.join(dir, f)); continue; }
  const c = spawnSync('ffmpeg', ['-v', 'info', '-i', path.join(dir, f), '-i', path.join(drawn, f), '-lavfi', 'blend=all_mode=difference,signalstats,metadata=print:file=-', '-f', 'null', '-'], {encoding: 'utf8'});
  const m = /YAVG=([\d.]+)/.exec(c.stdout + c.stderr);
  const avg = m ? Number(m[1]) : NaN;
  const over = !(avg <= limit);
  if (over) bad++;
  console.log(`${f}  mean diff ${Number.isNaN(avg) ? 'n/a' : avg.toFixed(4)}${over ? '  OVER LIMIT' : ''}`);
}
console.log(update ? `golden: updated ${frames.length} frames` : `golden: ${frames.length - bad}/${frames.length} within ${limit}`);
process.exit(bad ? 1 : 0);
