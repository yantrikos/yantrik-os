// Where characters stand and where the camera looks. All in 1920x1080 scene space.
import {slotX} from './stagepos.mjs';

export const W = 1920, H = 1080, FOOT_Y = 835;
export {slotX};

export const rigWidth = (n: number) => (n <= 1 ? 560 : n === 2 ? 520 : n === 3 ? 480 : 440);
export const rigHeight = (n: number) => rigWidth(n) * 1.5;
/** Screen y of a point given in rig coordinates (0..600). */
export const rigY = (n: number, ry: number) => FOOT_Y - (565 / 600) * rigHeight(n) + (ry / 600) * rigHeight(n);

export type Cam = {cx: number; cy: number; z: number};
export type ShotKind = 'wide' | 'two-shot' | 'close-up' | 'push-in';

const clamp = (v: number, a: number, b: number) => Math.min(b, Math.max(a, v));
export const fit = (c: Cam): Cam => {
  const z = Math.max(1, c.z);
  return {z, cx: clamp(c.cx, W / 2 / z, W - W / 2 / z), cy: clamp(c.cy, H / 2 / z, H - H / 2 / z)};
};

/**
 * Camera target for a shot. `xs` is where everybody stands right now, `speaker`
 * and `partner` are indices into it (partner: whoever the speaker is talking with).
 */
export function shot(kind: ShotKind, speaker: number, partner: number, xs: number[], progress: number): Cam {
  const n = xs.length;
  const sx = xs[speaker];
  const chest = rigY(n, 300);
  if (kind === 'wide' || (n === 1 && kind === 'two-shot')) return fit({cx: W / 2, cy: H / 2, z: 1});
  if (kind === 'push-in') {
    const k = 1 + 0.26 * progress;
    return fit({cx: W / 2 + (sx - W / 2) * 0.55 * progress, cy: H / 2 + (chest - H / 2) * 0.35 * progress, z: k});
  }
  if (kind === 'close-up') return fit({cx: sx, cy: rigY(n, 230), z: 2.15});
  const px = xs[partner];
  const span = Math.abs(sx - px) + rigWidth(n) * 1.15;
  return fit({cx: (sx + px) / 2, cy: rigY(n, 270), z: clamp(W / span, 1.2, 1.5)});
}

export const lerpCam = (a: Cam, b: Cam, t: number): Cam => ({cx: a.cx + (b.cx - a.cx) * t, cy: a.cy + (b.cy - a.cy) * t, z: a.z * Math.pow(b.z / a.z, t)});
export const ease = (t: number) => { const x = clamp(t, 0, 1); return x * x * (3 - 2 * x); };
