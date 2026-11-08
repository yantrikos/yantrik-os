// QA: render still frames of an episode. node scripts/frames.mjs <work-dir> <outdir> <frame...>
import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {bundleStage, findBrowser} from './bundle.mjs';
import {renderStill, selectComposition} from '@remotion/renderer';

const stage = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const [work, outDir, ...frames] = process.argv.slice(2);
const episode = JSON.parse(fs.readFileSync(path.join(work, 'episode.json'), 'utf8'));
const timeline = JSON.parse(fs.readFileSync(path.join(work, 'timeline.json'), 'utf8'));
fs.mkdirSync(path.join(work, 'public', 'fonts'), {recursive: true});
fs.copyFileSync(path.join(stage, 'fonts', 'Fredoka.ttf'), path.join(work, 'public', 'fonts', 'Fredoka.ttf'));
fs.mkdirSync(outDir, {recursive: true});
const browserExecutable = findBrowser(stage);
const serveUrl = await bundleStage(stage, path.join(work, 'public'));
const inputProps = {episode, timeline};
const composition = await selectComposition({serveUrl, id: 'Episode', inputProps, browserExecutable});
for (const f of frames) {
  await renderStill({composition, browserExecutable, serveUrl, inputProps, frame: Number(f), output: path.join(outDir, `f${String(f).padStart(5, '0')}.png`)});
}
console.log('frames done');
