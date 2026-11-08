// QA: render still frames of an episode. node scripts/frames.mjs <work-dir> <outdir> <frame...>
// Writes <outdir>/fNNNNN.png (lossless). These are the goldens other renderers are diffed against.
import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {buildPage, findBrowser, launch, filmFrames} from './page.mjs';

const stage = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const [work, outDir, ...frames] = process.argv.slice(2);
const episode = JSON.parse(fs.readFileSync(path.join(work, 'episode.json'), 'utf8'));
const timeline = JSON.parse(fs.readFileSync(path.join(work, 'timeline.json'), 'utf8'));
fs.mkdirSync(outDir, {recursive: true});
const exe = findBrowser(stage);
if (!exe) { console.error('frames: the render browser is not installed (install.sh).'); process.exit(1); }
const pageDir = path.join(work, 'page'), scratch = path.join(work, 'tmp-frames');
process.on('exit', () => { for (const d of [pageDir, scratch]) fs.rmSync(d, {recursive: true, force: true}); });
await buildPage(stage, pageDir);
const browser = await launch(exe, scratch);
const nums = frames.map(Number);
try {
  await filmFrames({
    browser, pageDir, workers: Math.max(1, Math.min(3, nums.length)), frames: nums, shot: {type: 'png'},
    show: {kind: 'episode', props: {episode, timeline}, fps: timeline.fps, durationInFrames: timeline.totalFrames},
    sink: (img, i) => fs.writeFileSync(path.join(outDir, `f${String(nums[i]).padStart(5, '0')}.png`), img),
  });
} finally { await browser.close(); }
console.log('frames done');
