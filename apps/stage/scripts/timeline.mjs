// Lays an episode out in frames: cold open, title card, scenes, credits, tag.
// One function serves both the lint (which guesses how long each line takes)
// and the voice step (which knows), so the estimate and the film cannot drift
// apart in anything except the speech itself.
import craft from '../src/stagecraft.mjs';
import {slotX, markX, freeSpot, walkFrames} from '../src/stagepos.mjs';

export const FPS = 30, TITLE = 75, CREDITS = 90, LEAD_IN = 3, SCENE_LEAD = 8, SCENE_TAIL = 10;
const LAUGH_FRAMES = 30;   // a laugh track keeps the scene alive at least this long after the line
const WALK_SETTLE = 6;

const cloneState = (m) => Object.fromEntries(Object.entries(m ?? {}).map(([k, v]) => [k, {...v}]));

/**
 * @param ep       normalised episode (scenes already in play order is not needed; parts are sorted here)
 * @param durOf    (line) => seconds of speech for that line
 */
export function layoutTimeline(ep, durOf) {
  const part = (p) => ep.scenes.filter((s) => (s.part ?? 'main') === p);
  const cold = part('cold-open'), main = part('main'), tag = part('tag');
  const propState = {};   // set -> prop -> {state, text}; survives from scene to scene
  const placed = {};      // set -> name -> x; where characters were last left
  const sfx = [];
  let frame = 0;

  const layScene = (sc, exit) => {
    const start = frame;
    frame += SCENE_LEAD;
    const ps = (propState[sc.set] ??= {});
    for (const p of sc.props ?? []) ps[p.prop] = {...ps[p.prop], ...pick(p)};
    const propsStart = cloneState(ps);
    const pl = (placed[sc.set] ??= {});
    const n = sc.present.length;
    const startX = Object.fromEntries(sc.present.map((nm, i) => [nm, pl[nm] ?? slotX(i, n)]));
    const cur = {...startX};
    const propEvents = [], moves = [];
    const lines = sc.lines.map((ln) => {
      const begin = frame;
      const holdF = Math.round((ln.hold ?? 0) * FPS);
      const lstart = begin + holdF + LEAD_IN;
      const aframes = Math.ceil(durOf(ln) * FPS);
      let end = lstart + aframes + Math.round(ln.pause * FPS);
      for (const p of ln.props ?? []) {
        ps[p.prop] = {...ps[p.prop], ...pick(p)};
        propEvents.push({at: lstart, prop: p.prop, ...pick(p)});
      }
      for (const [name, mark] of Object.entries(ln.move ?? {})) {
        if (!(name in cur)) continue;
        const home = slotX(sc.present.indexOf(name), n);
        const want = markX(sc.set, mark, home);
        if (want === null) continue;
        const to = mark === 'home' ? home : freeSpot(want, Object.entries(cur).filter(([k]) => k !== name).map(([, x]) => x));
        const frames = walkFrames(cur[name], to);
        if (Math.abs(to - cur[name]) < 4) continue;
        moves.push({name, from: cur[name], to, start: lstart, frames});
        cur[name] = to;
        end = Math.max(end, lstart + frames + WALK_SETTLE);
      }
      for (const name of ln.sfx ?? []) {
        const at = (craft.sfx[name]?.at ?? 'start') === 'end' ? lstart + aframes + 2 : lstart;
        sfx.push({name, at, volume: craft.sfx[name]?.volume ?? 0.5});
      }
      if (ln.laugh) {
        const at = lstart + aframes + 3;
        sfx.push({name: 'laugh', at, volume: craft.sfx.laugh.volume});
        end = Math.max(end, at + LAUGH_FRAMES);
      }
      frame = end;
      const out = {...ln, begin, holdFrames: holdF, start: lstart, audioFrames: aframes, end};
      Object.defineProperty(out, 'src', {value: ln, enumerable: false});
      return out;
    });
    Object.assign(pl, cur);
    frame += SCENE_TAIL;
    return {...sc, start, end: frame, exit, propsStart, propEvents, startX, moves, lines};
  };

  const run = (list, last) => list.map((sc, i) => layScene(sc, i < list.length - 1 ? list[i + 1].transition : last));
  const scenes = [];
  scenes.push(...run(cold, 'fade'));
  const titleStart = frame;
  frame += TITLE;
  scenes.push(...run(main, 'cut'));
  const creditsStart = frame;
  frame += CREDITS;
  scenes.push(...run(tag, 'fade'));
  return {fps: FPS, titleStart, titleFrames: TITLE, creditsStart, creditsFrames: CREDITS, totalFrames: frame, scenes, sfx, music: null};
}

function pick(p) {
  const o = {};
  if (p.state !== undefined) o.state = p.state;
  if (p.text !== undefined) o.text = p.text;
  return o;
}
