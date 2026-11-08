// The built-in sound cues, synthesised here from nothing (no samples, no
// network, nothing to license). Deterministic: the same name is the same
// bytes every time. Cues: record-beep, rimshot, power-down, alarm, door,
// typing, coffee-pour, crowd-gasp, and laugh (the optional laugh track).
import fs from 'node:fs';

const SR = 44100;
const TAU = Math.PI * 2;

function rngMaker(seed) { let s = seed >>> 0; return () => ((s = (s * 1664525 + 1013904223) >>> 0) / 4294967296); }
const buf = (sec) => new Float32Array(Math.ceil(sec * SR));
const env = (t, a, d) => Math.min(1, t / a) * Math.exp(-t * d);

/** Two-pole band-pass (RBJ biquad), in place. */
function bandpass(x, freq, q) {
  const w = (TAU * freq) / SR, al = Math.sin(w) / (2 * q), a0 = 1 + al;
  const b0 = al / a0, b2 = -al / a0, a1 = (-2 * Math.cos(w)) / a0, a2 = (1 - al) / a0;
  let x1 = 0, x2 = 0, y1 = 0, y2 = 0;
  for (let i = 0; i < x.length; i++) {
    const y = b0 * x[i] + b2 * x2 - a1 * y1 - a2 * y2;
    x2 = x1; x1 = x[i]; y2 = y1; y1 = y; x[i] = y;
  }
  return x;
}
function lowpass(x, freq) {
  const a = 1 - Math.exp((-TAU * freq) / SR);
  let y = 0;
  for (let i = 0; i < x.length; i++) { y += a * (x[i] - y); x[i] = y; }
  return x;
}
const noise = (n, r) => { const o = new Float32Array(n); for (let i = 0; i < n; i++) o[i] = r() * 2 - 1; return o; };
const mixInto = (dst, src, at, gain = 1) => { const s0 = Math.floor(at * SR); for (let i = 0; i < src.length && s0 + i < dst.length; i++) dst[s0 + i] += src[i] * gain; };

const tone = (sec, f, amp, d, kind = 'sine') => {
  const o = buf(sec);
  for (let i = 0; i < o.length; i++) {
    const t = i / SR, p = (f * t) % 1;
    const v = kind === 'square' ? (p < 0.5 ? 1 : -1) * 0.6 + Math.sin(TAU * f * t) * 0.4 : Math.sin(TAU * f * t);
    o[i] = v * amp * env(t, 0.004, d);
  }
  return o;
};

const drum = (sec, f0, f1, d, r) => {
  const o = buf(sec);
  let ph = 0;
  for (let i = 0; i < o.length; i++) {
    const t = i / SR;
    ph += (TAU * (f1 + (f0 - f1) * Math.exp(-t * 28))) / SR;
    o[i] = (Math.sin(ph) * 0.9 + (r() * 2 - 1) * 0.25 * Math.exp(-t * 60)) * env(t, 0.002, d);
  }
  return o;
};

const MAKERS = {
  'record-beep': () => {
    // the camera-is-rolling double beep: a short low one, a longer high one
    const o = buf(0.9);
    mixInto(o, tone(0.16, 880, 0.8, 5, 'square'), 0.02);
    mixInto(o, tone(0.34, 1320, 0.8, 3.5, 'square'), 0.26);
    return o;
  },
  rimshot: () => {
    const r = rngMaker(11), o = buf(1.6);
    mixInto(o, drum(0.3, 190, 120, 14, r), 0.0);
    mixInto(o, drum(0.3, 150, 100, 14, r), 0.17);
    const crash = lowpass(noise(Math.ceil(1.2 * SR), r), 9000).map((v, i) => v * Math.exp(-(i / SR) * 4.5));
    mixInto(o, bandpass(crash, 7000, 0.6), 0.36, 1.3);
    return o;
  },
  'power-down': () => {
    const o = buf(1.9);
    let ph = 0;
    for (let i = 0; i < o.length; i++) {
      const t = i / SR, f = 40 + 900 * Math.exp(-t * 2.3);
      ph += (TAU * f) / SR;
      o[i] = (Math.sin(ph) * 0.7 + Math.sin(ph * 2.01) * 0.2 + Math.sin(ph * 0.5) * 0.2) * Math.min(1, t / 0.01) * Math.max(0, 1 - t / 1.85) ** 1.6;
    }
    return o;
  },
  alarm: () => {
    const o = buf(2.0);
    let ph = 0;
    for (let i = 0; i < o.length; i++) {
      const t = i / SR, f = Math.floor(t / 0.3) % 2 ? 640 : 860;
      ph += (TAU * f) / SR;
      o[i] = (Math.sign(Math.sin(ph)) * 0.5 + Math.sin(ph) * 0.5) * Math.min(1, t / 0.02) * Math.min(1, (2.0 - t) / 0.15) * 0.7;
    }
    return lowpass(o, 3800);
  },
  door: () => {
    const r = rngMaker(5), o = buf(1.1);
    mixInto(o, lowpass(noise(Math.ceil(0.5 * SR), r), 380).map((v, i) => v * env(i / SR, 0.002, 12) * 3.2), 0.0);
    mixInto(o, tone(0.5, 62, 0.9, 11), 0.0);
    mixInto(o, bandpass(noise(Math.ceil(0.05 * SR), r), 2600, 3).map((v, i) => v * env(i / SR, 0.001, 60) * 5), 0.08);
    mixInto(o, bandpass(noise(Math.ceil(0.04 * SR), r), 1900, 4).map((v, i) => v * env(i / SR, 0.001, 70) * 4), 0.55);
    return o;
  },
  typing: () => {
    const r = rngMaker(23), o = buf(1.6);
    let t = 0.04;
    while (t < 1.45) {
      const click = bandpass(noise(Math.ceil(0.03 * SR), r), 1800 + r() * 2200, 2.5).map((v, i) => v * env(i / SR, 0.0008, 90) * 6);
      mixInto(o, click, t, 0.7 + r() * 0.4);
      t += 0.05 + r() * 0.11 + (r() < 0.12 ? 0.12 : 0);
    }
    return o;
  },
  'coffee-pour': () => {
    const r = rngMaker(31), o = buf(2.2);
    const base = bandpass(noise(o.length, r), 2400, 0.9);
    for (let i = 0; i < o.length; i++) {
      const t = i / SR, a = Math.min(1, t / 0.15) * Math.min(1, (2.2 - t) / 0.4);
      const wob = 0.65 + 0.35 * Math.sin(TAU * 7 * t + Math.sin(TAU * 1.3 * t) * 2);
      o[i] = base[i] * a * wob * 0.9;
    }
    let t = 0.5;
    while (t < 1.9) { mixInto(o, tone(0.07, 330 + r() * 260, 0.35, 40), t); t += 0.08 + r() * 0.15; }
    return o;
  },
  'crowd-gasp': () => {
    const r = rngMaker(41), n = Math.ceil(1.1 * SR);
    const x = noise(n, r);
    const a = bandpass(x.slice(), 900, 1.4), b = bandpass(x.slice(), 2300, 2);
    const o = buf(1.1);
    for (let i = 0; i < n; i++) {
      const t = i / SR, e = Math.min(1, t / 0.45) ** 1.5 * Math.exp(-Math.max(0, t - 0.5) * 7);
      o[i] = (a[i] * 1.2 + b[i] * 0.7) * e * 2.2;
    }
    return o;
  },
  laugh: () => {
    // eight people, each a run of "ha"s with their own pitch and timing
    const r = rngMaker(53), o = buf(2.3);
    for (let p = 0; p < 8; p++) {
      const f0 = 150 + r() * 220, t0 = r() * 0.25, has = 4 + Math.floor(r() * 3), gap = 0.15 + r() * 0.06;
      for (let k = 0; k < has; k++) {
        const t = t0 + k * gap, f = f0 * (1 - k * 0.025), len = 0.11;
        const ha = buf(len);
        for (let i = 0; i < ha.length; i++) {
          const tt = i / SR; let v = 0;
          for (let h = 1; h <= 14; h++) v += Math.sin(TAU * f * h * tt + h) / h;
          ha[i] = (v + (r() * 2 - 1) * 0.25) * Math.sin((Math.PI * i) / ha.length) ** 1.5;
        }
        bandpass(ha, 900 + r() * 400, 1.1);
        mixInto(o, ha, t, (1 - k / (has + 1)) * (0.5 + r() * 0.5) * 2.4);
      }
    }
    for (let i = 0; i < o.length; i++) o[i] *= Math.min(1, i / (0.1 * SR)) * Math.min(1, (o.length - i) / (0.7 * SR));
    return o;
  },
};

export const SFX_NAMES = Object.keys(MAKERS);

/** Writes <file> as 16-bit mono WAV, peak-normalised to about -3 dB. */
export function renderSfx(name, file) {
  const make = MAKERS[name];
  if (!make) throw new Error(`no sound cue called '${name}'. Choose from: ${SFX_NAMES.join(', ')}.`);
  const x = make();
  let peak = 0;
  for (const v of x) peak = Math.max(peak, Math.abs(v));
  const g = peak > 0 ? 0.7 / peak : 1;
  const pcm = Buffer.alloc(44 + x.length * 2);
  pcm.write('RIFF', 0); pcm.writeUInt32LE(36 + x.length * 2, 4); pcm.write('WAVEfmt ', 8);
  pcm.writeUInt32LE(16, 16); pcm.writeUInt16LE(1, 20); pcm.writeUInt16LE(1, 22);
  pcm.writeUInt32LE(SR, 24); pcm.writeUInt32LE(SR * 2, 28); pcm.writeUInt16LE(2, 32); pcm.writeUInt16LE(16, 34);
  pcm.write('data', 36); pcm.writeUInt32LE(x.length * 2, 40);
  for (let i = 0; i < x.length; i++) pcm.writeInt16LE(Math.max(-32767, Math.min(32767, Math.round(x[i] * g * 32767))), 44 + i * 2);
  fs.writeFileSync(file, pcm);
}
