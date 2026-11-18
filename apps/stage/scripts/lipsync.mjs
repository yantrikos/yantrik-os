// Lip-sync from audio alone. Piper's CLI does not emit phoneme timings, so
// each 30 fps frame is classified from the amplitude envelope plus two cheap
// spectral cues (zero-crossing rate and a brightness estimate):
//   quiet                     -> rest
//   loud + open vowel         -> A      mid + dark -> O/U    mid + bright -> E
//   hiss (high ZCR)           -> F      sudden dip inside speech -> M
// Output per frame: a viseme letter and a 0..1 loudness for body energy.
import {spawnSync} from 'node:child_process';

const SR = 16000;

export function decodeMono(file) {
  const r = spawnSync('ffmpeg', ['-v', 'error', '-i', file, '-f', 'f32le', '-ar', String(SR), '-ac', '1', '-'], {maxBuffer: 1 << 28});
  if (r.status !== 0) throw new Error(`ffmpeg could not decode ${file}: ${r.stderr}`);
  const b = r.stdout;
  return new Float32Array(b.buffer.slice(b.byteOffset, b.byteOffset + b.byteLength - (b.byteLength % 4)));
}

export function lipsync(file, fps = 30) {
  const x = decodeMono(file);
  const hop = Math.round(SR / fps), win = hop * 2;
  const n = Math.max(1, Math.ceil(x.length / hop));
  const rms = new Float32Array(n), zcr = new Float32Array(n), bright = new Float32Array(n);
  for (let f = 0; f < n; f++) {
    const s = Math.max(0, Math.floor(f * hop - hop / 2)), e = Math.min(x.length, s + win);
    let sum = 0, z = 0, d = 0, cnt = 0;
    for (let i = s; i < e; i++) {
      sum += x[i] * x[i];
      if (i > s) { if ((x[i] >= 0) !== (x[i - 1] >= 0)) z++; d += Math.abs(x[i] - x[i - 1]); }
      cnt++;
    }
    rms[f] = Math.sqrt(sum / Math.max(1, cnt));
    zcr[f] = z / Math.max(1, cnt) * SR / 2; // ~dominant frequency in Hz
    bright[f] = d / Math.max(1e-6, Math.sqrt(sum * Math.max(1, cnt))); // first-difference / level: high = bright
  }
  const sorted = [...rms].sort((a, b) => a - b);
  const p90 = sorted[Math.floor(n * 0.9)] || 1e-3;
  const loud = Array.from(rms, (v) => Math.min(1, v / (p90 * 1.15)));
  const quiet = 0.07;
  const raw = [];
  for (let f = 0; f < n; f++) {
    const L = loud[f];
    if (L < quiet) { raw.push('rest'); continue; }
    const prev = loud[f - 1] ?? 0, next = loud[f + 1] ?? 0;
    const dip = Math.min(loud[f - 2] ?? 0, loud[f + 2] ?? 0);
    if (L < 0.26 && (prev > 0.4 || next > 0.4) && dip > 0.3) { raw.push('M'); continue; }
    if (zcr[f] > 3200 && L < 0.6) { raw.push('F'); continue; }
    if (L > 0.82 && bright[f] < 0.6) { raw.push('A'); continue; }
    if (bright[f] > 0.5 || zcr[f] > 1700) raw.push('E');
    else raw.push(L > 0.5 ? 'O' : 'U');
  }
  // smooth: drop single-frame flickers, but keep closures and rests.
  const out = raw.slice();
  for (let f = 1; f < n - 1; f++) {
    if (raw[f] !== raw[f - 1] && raw[f] !== raw[f + 1] && raw[f - 1] === raw[f + 1] && raw[f] !== 'rest' && raw[f] !== 'M') out[f] = raw[f - 1];
  }
  // light envelope smoothing for body energy
  const energy = loud.map((v, i) => Math.round(((loud[i - 1] ?? v) * 0.25 + v * 0.5 + (loud[i + 1] ?? v) * 0.25) * 100) / 100);
  return {frames: n, durationSec: x.length / SR, visemes: out, energy};
}
