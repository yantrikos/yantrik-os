// Where characters stand, in 1920x1080 scene space. Plain JS so both the
// timeline (node) and the template (browser) use the very same numbers.
import craft from './stagecraft.mjs';

export const W = 1920;
export const WALK_PX_PER_FRAME = 9;   // 270 px a second, an easy stroll
export const MIN_WALK_FRAMES = 14;
export const STRIDE_PX = 64;          // one step; sets the leg rhythm
export const MARK_GAP = 170;          // closest two characters may stand

/** The home spot of the i-th of n characters in a row. */
export function slotX(i, n) {
  if (n === 1) return W / 2;
  const left = n === 2 ? 640 : n === 3 ? 380 : 270, right = W - left;
  return left + (i * (right - left)) / (n - 1);
}

/** Every mark name a set accepts (the shared ones plus its own). */
export function marksOf(set) {
  return {...craft.everyMarks, ...(craft.sets[set]?.marks ?? {})};
}

/** The x of a named mark; 'home' is the character's own slot. null if unknown. */
export function markX(set, mark, homeX) {
  if (mark === 'home') return homeX;
  const x = marksOf(set)[mark];
  return x === undefined ? null : x;
}

/** Nudge a target so it does not land on top of someone standing nearby. */
export function freeSpot(x, others) {
  let best = x;
  for (let k = 0; k < 6; k++) {
    if (others.every((o) => Math.abs(o - best) >= MARK_GAP)) return best;
    const side = best >= W / 2 ? -1 : 1;
    best = x + side * MARK_GAP * Math.ceil((k + 1) / 2) * (k % 2 ? -1 : 1);
    best = Math.min(W - 160, Math.max(160, best));
  }
  return x;
}

export const walkFrames = (from, to) => Math.max(MIN_WALK_FRAMES, Math.ceil(Math.abs(to - from) / WALK_PX_PER_FRAME));
