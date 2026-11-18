// Shared by every prop: a smooth 0..1 for how lit a lamp is, and text that
// shrinks to fit a plate.
import React from 'react';

const clamp = (v: number, a = 0, b = 1) => Math.min(b, Math.max(a, v));

/** How bright a beacon is right now. blink: 1 s cycle, on a bit longer than off. steady: always. off: dark. */
export function beacon(state: string, frame: number): number {
  if (state === 'off') return 0.05;
  if (state === 'steady' || state === 'on') return 1;
  const ph = frame % 30;
  if (ph < 14) return 1;
  if (ph < 17) return 1 - 0.93 * ((ph - 13) / 3);
  if (ph < 28) return 0.07;
  return 0.07 + 0.93 * ((ph - 27) / 3);
}

/** A one-off ring that swells out of a point for ~14 frames after a change. */
export function Pulse({since, frame, r0, r1, color}: {since: number; frame: number; r0: number; r1: number; color: string}) {
  const k = (frame - since) / 14;
  if (k < 0 || k >= 1) return null;
  return <circle r={r0 + (r1 - r0) * k} fill="none" stroke={color} strokeWidth={10 * (1 - k) + 1} opacity={0.85 * (1 - k)} />;
}

/** Font size that fits `text` into `width` (average Fredoka glyph is ~0.6 em), within [min, max]. */
export const fitSize = (text: string, width: number, max: number, min = 12) => clamp(width / (Math.max(1, text.length) * 0.6), min, max);

export const FONT = 'Fredoka, sans-serif';
export const clamp01 = clamp;
export const hash = (n: number) => { const x = Math.sin(n * 127.1 + 311.7) * 43758.5453; return x - Math.floor(x); };
