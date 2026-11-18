// The sound track: one ffmpeg filter graph built from timeline.json. Every
// voice line and sound effect is delayed to its frame (adelay) and the music
// bed is ducked while someone talks; amix sums them. Each input is padded to
// the film's length first, so amix always divides by the same count and one
// volume stage undoes it.
import path from 'node:path';

const RATE = 44100;
const DUCK = 0.42, DUCK_REACH = 6;   // the bed drops to 42% from 6 frames before a line to 6 frames after

/** Talking spans in seconds, widened by DUCK_REACH frames and merged. */
export function duckSpans(timeline) {
  const fps = timeline.fps;
  const spans = [];
  for (const s of timeline.scenes) for (const l of s.lines) spans.push([(l.start - DUCK_REACH) / fps, (l.start + l.audioFrames + DUCK_REACH) / fps]);
  spans.sort((a, b) => a[0] - b[0]);
  const merged = [];
  for (const sp of spans) {
    const last = merged[merged.length - 1];
    if (last && sp[0] <= last[1]) last[1] = Math.max(last[1], sp[1]); else merged.push([...sp]);
  }
  return merged;
}

/** {inputs: [files], graph: filter_complex text, map: output label} for the whole film. */
export function buildAudio(timeline, publicDir) {
  const fps = timeline.fps, total = timeline.totalFrames / fps;
  const inputs = [], chains = [];
  const add = (file, delaySec, volume, trimSec, extra = '') => {
    const i = inputs.length;
    inputs.push(path.join(publicDir, file));
    const ms = Math.max(0, Math.round(delaySec * 1000));
    const trim = trimSec ? `atrim=0:${trimSec.toFixed(4)},` : '';
    chains.push(`[${i + 1}:a]aformat=sample_fmts=fltp:sample_rates=${RATE}:channel_layouts=mono,${trim}adelay=${ms}:all=1,${extra}volume=${volume}:eval=once,apad=whole_dur=${total.toFixed(4)},atrim=0:${total.toFixed(4)}[a${i}]`);
  };
  for (const s of timeline.scenes) for (const l of s.lines) add(l.audio, l.start / fps, 1, (l.audioFrames + 2) / fps);
  for (const c of timeline.sfx ?? []) add(c.file, c.at / fps, c.volume ?? 1, 0);
  if (timeline.music) {
    const base = (timeline.music.volume ?? 0.5) * 0.5;
    const spans = duckSpans(timeline);
    const near = spans.map(([a, b]) => `between(t,${a.toFixed(4)},${b.toFixed(4)})`).join('+') || '0';
    add(timeline.music.file, 0, 1, 0, `volume='${base}*(1-${(1 - DUCK).toFixed(2)}*gt(${near},0))':eval=frame,`);
  }
  const n = inputs.length;
  const mix = chains.length ? `${inputs.map((_, i) => `[a${i}]`).join('')}amix=inputs=${n}:duration=longest:dropout_transition=0,volume=${n}[mix]` : null;
  return {inputs, graph: chains.concat(mix ? [mix] : []).join(';\n'), map: mix ? '[mix]' : null};
}
