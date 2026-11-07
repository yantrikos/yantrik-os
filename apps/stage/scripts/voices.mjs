#!/usr/bin/env node
// Voice step: speak every line with the character's Piper voice, shape it
// (pitch, formant, EQ), measure the mouth shapes, lay out the timeline, and
// synthesise the music bed. Results are cached by content, so a re-run with
// the same text costs nothing.
// Usage: voices.mjs <normalized-episode.json> <work-dir>
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import {spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import {lipsync} from './lipsync.mjs';
import {renderMusic} from './music.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const stage = path.resolve(here, '..');
const FPS = 30, TITLE = 75, CREDITS = 90, LEAD_IN = 3, SCENE_LEAD = 8, SCENE_TAIL = 10;

const [epFile, work] = process.argv.slice(2);
if (!epFile || !work) { console.error('usage: voices.mjs <normalized-episode.json> <work-dir>'); process.exit(2); }
const ep = JSON.parse(fs.readFileSync(epFile, 'utf8'));
const bank = JSON.parse(fs.readFileSync(path.join(stage, 'voices', 'voices.json'), 'utf8'));
const piperDir = process.env.STAGE_PIPER_DIR || '/opt/yantrik/models/tts';
const modelDirs = [path.join(stage, 'voices', 'models'), piperDir];
const cache = path.join(path.dirname(work), 'cache');
const pub = path.join(work, 'public');
for (const d of [cache, path.join(pub, 'voice')]) fs.mkdirSync(d, {recursive: true});

const die = (m) => { console.error(`stage voices: ${m}`); process.exit(1); };
const findModel = (name) => {
  const f = bank.models[name]?.file;
  for (const d of modelDirs) if (f && fs.existsSync(path.join(d, f))) return path.join(d, f);
  die(`the voice model '${name}' (${f}) is not installed. Run voices/fetch-voices.sh once at install time.`);
};
const piperBin = path.join(piperDir, 'piper');
if (!fs.existsSync(piperBin)) die(`Piper is not at ${piperBin}. Set STAGE_PIPER_DIR to the folder holding the piper program.`);
const run = (cmd, args, opts = {}) => {
  const r = spawnSync(cmd, args, {encoding: 'utf8', ...opts});
  if (r.status !== 0) die(`${cmd} failed: ${(r.stderr || '').split('\n').slice(-4).join(' ')}`);
  return r;
};
const voiceByName = Object.fromEntries(ep.cast.map((c) => [c.name, c.voice]));

function speak(text, voiceKey) {
  const v = bank.voices[voiceKey];
  if (!v) die(`voice '${voiceKey}' is not in voices.json.`);
  const key = crypto.createHash('sha1').update(JSON.stringify([bank.version, text, v])).digest('hex').slice(0, 16);
  const wav = path.join(cache, `${key}.wav`), meta = path.join(cache, `${key}.json`);
  if (fs.existsSync(wav) && fs.existsSync(meta)) return {key, wav, ...JSON.parse(fs.readFileSync(meta, 'utf8')), cached: true};
  const model = findModel(v.model);
  const sr = JSON.parse(fs.readFileSync(model + '.json', 'utf8')).audio.sample_rate;
  const raw = path.join(cache, `${key}.raw.wav`);
  const args = ['--model', model, '--output_file', raw, '--espeak_data', path.join(piperDir, 'espeak-ng-data'), '--quiet',
    '--length_scale', String(v.length_scale), '--noise_scale', String(v.noise_scale), '--noise_w', String(v.noise_w), '--sentence_silence', '0.18'];
  if (v.speaker !== undefined) args.push('--speaker', String(v.speaker));
  run(piperBin, args, {input: text + '\n', env: {...process.env, LD_LIBRARY_PATH: `${piperDir}:${process.env.LD_LIBRARY_PATH || ''}`}});
  // pitch + formant shift (rate trick), then voice EQ, loudness, and a hair of tail
  const p = v.pitch ?? 1;
  const af = [`asetrate=${Math.round(sr * p)}`, `aresample=${bank.sample_rate}`, `atempo=${(1 / p).toFixed(4)}`, v.fx, 'loudnorm=I=-17:TP=-2:LRA=8', 'apad=pad_dur=0.08']
    .filter(Boolean).join(',');
  run('ffmpeg', ['-y', '-v', 'error', '-i', raw, '-af', af, '-ar', String(bank.sample_rate), '-ac', '1', wav]);
  fs.rmSync(raw, {force: true});
  const ls = lipsync(wav, FPS);
  const rec = {frames: ls.frames, durationSec: ls.durationSec, visemes: ls.visemes.map((x) => (x === 'rest' ? '.' : x)).join(''), energy: ls.energy};
  fs.writeFileSync(meta, JSON.stringify(rec));
  return {key, wav, ...rec, cached: false};
}

let frame = TITLE, spoken = 0, made = 0, hits = 0;
const scenes = ep.scenes.map((sc) => {
  const start = frame;
  frame += SCENE_LEAD;
  const lines = sc.lines.map((ln) => {
    const r = speak(ln.text, voiceByName[ln.speaker]);
    r.cached ? hits++ : made++;
    fs.copyFileSync(r.wav, path.join(pub, 'voice', `${r.key}.wav`));
    const lstart = frame + LEAD_IN;
    const aframes = Math.ceil(r.durationSec * FPS);
    const pauseF = Math.round(ln.pause * FPS);
    const vis = r.visemes;
    const first = vis.search(/[^.]/), last = vis.length - 1 - [...vis].reverse().findIndex((c) => c !== '.');
    frame = lstart + aframes + pauseF;
    spoken += r.durationSec;
    return {speaker: ln.speaker, text: ln.text, emotion: ln.emotion, pose: ln.pose ?? null, camera: ln.camera ?? null, react: ln.react,
      start: lstart, audioFrames: aframes, end: frame, audio: `voice/${r.key}.wav`, visemes: vis, energy: r.energy,
      speakStart: Math.max(0, first), speakEnd: Math.max(first, last)};
  });
  frame += SCENE_TAIL;
  return {set: sc.set, camera: sc.camera, transition: sc.transition, present: sc.present, start, end: frame, lines};
});
const creditsStart = frame;
const total = frame + CREDITS;

let music = null;
if (ep.music.cue !== 'none') {
  const mfile = path.join(pub, 'music.wav');
  renderMusic(ep.music.cue, total / FPS + 1, mfile);
  music = {file: 'music.wav', volume: ep.music.volume};
}
const timeline = {fps: FPS, titleFrames: TITLE, creditsStart, creditsFrames: CREDITS, totalFrames: total, scenes, music};
fs.writeFileSync(path.join(work, 'timeline.json'), JSON.stringify(timeline));
console.log(`voices: ${ep.scenes.reduce((n, s) => n + s.lines.length, 0)} lines (${made} spoken, ${hits} from cache), ${spoken.toFixed(1)}s of speech, ${(total / FPS).toFixed(1)}s episode, music ${ep.music.cue}`);
