// Where each character is at a given frame: standing at their slot or last
// mark, or striding between two marks.
import {ease} from './layout';
import {STRIDE_PX} from './stagepos.mjs';

export type Move = {name: string; from: number; to: number; start: number; frames: number};
export type Placed = {x: number; walking: boolean; dir: number; phase: number};

export function placeAt(name: string, startX: number, moves: Move[], g: number): Placed {
  let x = startX;
  for (const m of moves) {
    if (m.name !== name || g < m.start) continue;
    const k = (g - m.start) / m.frames;
    if (k < 1) {
      const e = ease(k), here = m.from + (m.to - m.from) * e;
      return {x: here, walking: true, dir: Math.sign(m.to - m.from), phase: (Math.abs(here - m.from) / STRIDE_PX) * Math.PI};
    }
    x = m.to;
  }
  return {x, walking: false, dir: 0, phase: 0};
}
