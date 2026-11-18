// A small royalty-free synth bed, generated here (no samples, no network).
// Plucks, a round bass, a soft pad and a light shaker, all deterministic.
// Cues: cheeky | calm | tense | triumphant.
import fs from 'node:fs';

const SR = 44100;
const MIDI = (n) => 440 * Math.pow(2, (n - 69) / 12);
const CUES = {
  cheeky: {bpm: 112, root: 48, scale: [0, 2, 4, 7, 9], prog: [0, 9, 5, 7], minor: false, swing: 0.1, shaker: 1, pad: 0.5, pluck: 1},
  calm: {bpm: 74, root: 50, scale: [0, 2, 4, 7, 9], prog: [0, 5, 9, 7], minor: false, swing: 0, shaker: 0, pad: 1, pluck: 0.5},
  tense: {bpm: 96, root: 45, scale: [0, 3, 5, 7, 10], prog: [0, 0, 8, 7], minor: true, swing: 0, shaker: 1, pad: 0.7, pluck: 0.6},
  triumphant: {bpm: 124, root: 48, scale: [0, 2, 4, 7, 9], prog: [0, 5, 7, 0], minor: false, swing: 0, shaker: 1, pad: 0.8, pluck: 1},
};

function rngMaker(seed) { let s = seed >>> 0; return () => ((s = (s * 1664525 + 1013904223) >>> 0) / 4294967296); }

export function renderMusic(cue, seconds, file, gain = 1) {
  const c = CUES[cue];
  const total = Math.ceil(seconds * SR);
  const buf = new Float32Array(total);
  const rnd = rngMaker(7);
  const beat = 60 / c.bpm;
  const add = (t0, dur, f, amp, kind) => {
    const s0 = Math.floor(t0 * SR), n = Math.floor(dur * SR);
    for (let i = 0; i < n && s0 + i < total; i++) {
      const t = i / SR;
      let env, v;
      if (kind === 'pluck') { env = Math.exp(-t * 7) * Math.min(1, t * 400);
        v = Math.sin(2 * Math.PI * f * t) + 0.35 * Math.sin(4 * Math.PI * f * t) * Math.exp(-t * 14) + 0.12 * Math.sin(6 * Math.PI * f * t); }
      else if (kind === 'bass') { env = Math.exp(-t * 3.2) * Math.min(1, t * 200); v = Math.sin(2 * Math.PI * f * t) + 0.25 * Math.sin(4 * Math.PI * f * t); }
      else if (kind === 'pad') { const a = Math.min(1, t / 0.5), r = Math.min(1, (dur - t) / 0.5); env = a * Math.max(0, r);
        v = Math.sin(2 * Math.PI * f * t) + Math.sin(2 * Math.PI * f * 1.004 * t) * 0.8 + 0.3 * Math.sin(2 * Math.PI * f * 2 * t); }
      else { env = Math.exp(-t * 60); v = rnd() * 2 - 1; }
      buf[s0 + i] += v * env * amp;
    }
  };
  const bars = Math.ceil(seconds / (beat * 4));
  const third = c.minor ? 3 : 4;
  for (let bar = 0; bar < bars; bar++) {
    const t0 = bar * beat * 4;
    const deg = c.prog[bar % c.prog.length];
    const root = c.root + deg - (deg > 6 ? 12 : 0);
    // pad chord
    for (const iv of [0, third, 7]) add(t0, beat * 4, MIDI(root + 12 + iv), 0.05 * c.pad, 'pad');
    // bass: root on 1, fifth on 3, walk in between
    const bassNotes = [0, 0, 7, 4];
    for (let b = 0; b < 4; b++) add(t0 + b * beat, beat * 0.9, MIDI(root - 12 + bassNotes[b]), 0.2, 'bass');
    // pluck melody: syncopated pentatonic figure, repeats with variation per bar
    for (let e = 0; e < 8; e++) {
      const t = t0 + e * beat / 2 + (e % 2 ? c.swing * beat / 2 : 0);
      const pat = (bar * 5 + e * 3) % 7;
      if ((e === 1 || e === 4 || e === 6 || (bar % 2 && e === 3)) && c.pluck > 0.4) {
        const note = root + 12 + c.scale[pat % c.scale.length] + (pat > 4 ? 12 : 0);
        add(t, 0.5, MIDI(note), 0.09 * c.pluck, 'pluck');
      }
      if (c.shaker && e % 2 === 1) add(t, 0.08, 0, 0.018, 'noise');
    }
  }
  // gentle global fade in/out, soft clip, write 16-bit mono WAV
  const fi = SR * 1.0, fo = SR * 2.5;
  const out = Buffer.alloc(44 + total * 2);
  out.write('RIFF', 0); out.writeUInt32LE(36 + total * 2, 4); out.write('WAVEfmt ', 8); out.writeUInt32LE(16, 16);
  out.writeUInt16LE(1, 20); out.writeUInt16LE(1, 22); out.writeUInt32LE(SR, 24); out.writeUInt32LE(SR * 2, 28);
  out.writeUInt16LE(2, 32); out.writeUInt16LE(16, 34); out.write('data', 36); out.writeUInt32LE(total * 2, 40);
  let lp = 0;
  for (let i = 0; i < total; i++) {
    const g = Math.min(1, i / fi) * Math.min(1, (total - i) / fo);
    lp += (buf[i] - lp) * 0.35; // soften the top end
    out.writeInt16LE(Math.round(Math.tanh(lp * gain * g * 1.6) * 30000), 44 + i * 2);
  }
  fs.writeFileSync(file, out);
}
