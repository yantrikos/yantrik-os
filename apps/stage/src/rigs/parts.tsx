// Shared rig machinery: colour maths, 2-bone arms, the face (eyes, brows,
// mouth, effects), poses and the RigFrame that animates every character.
// Each character file only supplies its own silhouette art.
import React, {useId} from 'react';

export const EXPRESSIONS = ['neutral', 'happy', 'smug', 'shocked', 'angry', 'sad', 'confused', 'laughing'] as const;
export const VISEMES = ['rest', 'A', 'E', 'O', 'U', 'M', 'F'] as const;
export const POSES = ['idle', 'talk-gesture', 'point', 'facepalm'] as const;
export type Expression = (typeof EXPRESSIONS)[number];
export type Viseme = (typeof VISEMES)[number];
export type Pose = (typeof POSES)[number];
export type Pt = [number, number];

// ---------------------------------------------------------------- colour
const hex2 = (h: string): [number, number, number] => {
  const s = h.replace('#', '');
  const f = s.length === 3 ? s.split('').map((c) => c + c).join('') : s;
  return [parseInt(f.slice(0, 2), 16), parseInt(f.slice(2, 4), 16), parseInt(f.slice(4, 6), 16)];
};
export const mix = (a: string, b: string, t: number) => {
  const x = hex2(a), y = hex2(b);
  return '#' + x.map((v, i) => Math.round(v + (y[i] - v) * t).toString(16).padStart(2, '0')).join('');
};
export const lighten = (c: string, t: number) => mix(c, '#ffffff', t);
export const darken = (c: string, t: number) => mix(c, '#000000', t);

export type Palette = {main: string; accent: string; cloth: string; hair: string; outline: string; skin: string};
export const resolvePalette = (d: Palette, o?: Partial<Palette>): Palette => {
  const p = {...d, ...(o || {})};
  if (o?.main && !o.skin) p.skin = o.main;
  if (o?.main && !o.outline) p.outline = darken(o.main, 0.62);
  if (o?.main && !o.hair) p.hair = darken(o.main, 0.38);
  return p;
};

// ----------------------------------------------------------------- types
export type RigProps = {
  expression?: Expression;
  viseme?: Viseme;
  pose?: Pose;
  frame: number;
  fps: number;
  energy?: number; // 0..1 speech loudness, adds life while talking
  palette?: Partial<Palette>;
  accessory?: string;
  flip?: boolean;
  look?: number; // -1..1 pupil direction
  seed?: number;
  width?: number;
  shadow?: boolean;
  walk?: number; // walk-cycle phase in radians while striding between marks; leave out when standing
};
export type Ctx = {
  p: Palette; acc: string; t: number; energy: number; uid: string;
  url: (n: string) => string; // gradient reference
};
export type FaceSpec = {
  eyeDx: number; eyeY: number; eyeR: number; style: 'round' | 'shades' | 'wide';
  iris: [string, string]; lidColor: string; browColor: string; browW: number;
  mouthY: number; mouthW: number; lip: string; cheek: string; fx: Pt; // fx: effect anchor
  browGap: number;
};
export type RigSpec = {
  name: string; palette: Palette; accessories: string[]; accessory: string;
  shoulderL: Pt; shoulderR: Pt; headC: Pt; neck: Pt;
  arm: {l1: number; l2: number; w: number; hand: number; sleeve?: boolean; sleeveLen?: number};
  face: FaceSpec;
  defs: (c: Ctx) => React.ReactNode;
  legs: (c: Ctx) => React.ReactNode;
  body: (c: Ctx) => React.ReactNode;
  head: (c: Ctx) => React.ReactNode; // art behind the face, head-local coords
  front: (c: Ctx) => React.ReactNode; // art over the face (glasses, fringe), head-local
  front2?: (c: Ctx) => React.ReactNode; // over arms, body-space (scarf ends, tie)
  restHand?: (side: 'l' | 'r') => Pt | null; // optional custom idle hand spot, absolute
};

// ------------------------------------------------------------- gradients
export const lin = (id: string, light: string, base: string, dark: string, vertical = false) => (
  <linearGradient id={id} x1={vertical ? 0.5 : 0.15} y1={vertical ? 0 : 0} x2={vertical ? 0.5 : 0.85} y2={1}>
    <stop offset="0" stopColor={light} /><stop offset="0.5" stopColor={base} /><stop offset="1" stopColor={dark} />
  </linearGradient>
);
export const rad = (id: string, light: string, base: string, dark: string, cx = 0.35, cy = 0.3) => (
  <radialGradient id={id} cx={cx} cy={cy} r={0.85}>
    <stop offset="0" stopColor={light} /><stop offset="0.55" stopColor={base} /><stop offset="1" stopColor={dark} />
  </radialGradient>
);

// ------------------------------------------------------------------ arms
function ik(s: Pt, h: Pt, l1: number, l2: number, bend: number) {
  const dx = h[0] - s[0], dy = h[1] - s[1];
  const d = Math.min(Math.max(Math.hypot(dx, dy), Math.abs(l1 - l2) + 1), l1 + l2 - 0.5);
  const a = Math.atan2(dy, dx);
  const ang = Math.acos(Math.max(-1, Math.min(1, (l1 * l1 + d * d - l2 * l2) / (2 * l1 * d)))) * bend;
  const e: Pt = [s[0] + l1 * Math.cos(a + ang), s[1] + l1 * Math.sin(a + ang)];
  const hand: Pt = [s[0] + d * Math.cos(a), s[1] + d * Math.sin(a)];
  return {e, hand};
}

export function Arm({c, spec, s, target, bend, point}: {c: Ctx; spec: RigSpec; s: Pt; target: Pt; bend: number; point?: boolean}) {
  const {l1, l2, w, hand: hr, sleeve, sleeveLen} = spec.arm;
  const reach = Math.hypot(target[0] - s[0], target[1] - s[1]) / (spec.arm.l1 + spec.arm.l2);
  const st = Math.min(Math.max(reach, 1), 1.4); // noodle arms may stretch a little
  const {e, hand} = ik(s, target, l1 * st, l2 * st, bend);
  const mp = (a: Pt, b: Pt, k: number) => `${a[0] + (b[0] - a[0]) * k},${a[1] + (b[1] - a[1]) * k}`;
  const line = `M${s[0]},${s[1]} L${mp(s, e, 0.55)} Q${e[0]},${e[1]} ${mp(e, hand, 0.45)} L${hand[0]},${hand[1]}`;
  const sl = sleeveLen ?? 0.55;
  const sx = e[0] + (hand[0] - e[0]) * sl, sy = e[1] + (hand[1] - e[1]) * sl;
  const sleeveD = `M${s[0]},${s[1]} L${mp(s, e, 0.55)} Q${e[0]},${e[1]} ${mp(e, hand, Math.min(0.45, sl * 0.8))} L${sx},${sy}`;
  const ang = (Math.atan2(hand[1] - e[1], hand[0] - e[0]) * 180) / Math.PI;
  return (
    <g strokeLinecap="round" strokeLinejoin="round" fill="none">
      <path d={line} stroke={c.p.outline} strokeWidth={w + 7} />
      <path d={line} stroke={c.p.skin} strokeWidth={w} />
      <path d={line} stroke={lighten(c.p.skin, 0.25)} strokeWidth={w * 0.28} transform="translate(-2,-2)" opacity={0.5} />
      {sleeve && <>
        <path d={sleeveD} stroke={c.p.outline} strokeWidth={w + 11} />
        <path d={sleeveD} stroke={c.p.cloth} strokeWidth={w + 4} />
        <path d={sleeveD} stroke={lighten(c.p.cloth, 0.3)} strokeWidth={w * 0.25} transform="translate(-2,-3)" opacity={0.45} />
      </>}
      <g transform={`translate(${hand[0]},${hand[1]}) rotate(${ang})`}>
        {point && <path d={`M${hr * 0.4},0 L${hr * 2.1},0`} stroke={c.p.outline} strokeWidth={hr * 0.7 + 6} />}
        {point && <path d={`M${hr * 0.4},0 L${hr * 2.1},0`} stroke={c.p.skin} strokeWidth={hr * 0.7} />}
        <circle r={hr + 3} fill={c.p.outline} stroke="none" />
        <circle r={hr} fill={c.p.skin} stroke="none" />
        <circle cx={-hr * 0.3} cy={-hr * 0.35} r={hr * 0.38} fill="#fff" opacity={0.28} stroke="none" />
      </g>
    </g>
  );
}

// ----------------------------------------------------------------- poses
function handTargets(pose: Pose, t: number, energy: number, spec: RigSpec, walk?: number) {
  const L = spec.shoulderL, R = spec.shoulderR;
  const sway = Math.sin(t * 1.7) * 3;
  const rest = (s: Pt, side: number): Pt => [s[0] + side * 8 + sway * side, s[1] + (spec.arm.l1 + spec.arm.l2) * 0.97];
  const out = {l: rest(L, -1), r: rest(R, 1), bl: 1, br: -1, tilt: 0, dy: 0, lean: 0, point: false, squint: false, shrug: 0};
  if (spec.restHand) {
    const a = spec.restHand('l'), b = spec.restHand('r');
    if (a) out.l = [a[0] + sway, a[1]];
    if (b) out.r = [b[0] - sway, b[1]];
  }
  if (pose === 'talk-gesture') {
    const k = 0.6 + energy * 0.9;
    out.r = [R[0] + 62 + Math.sin(t * 5.1) * 18 * k, R[1] + 28 - Math.abs(Math.sin(t * 3.4)) * 70 * k];
    out.l = [L[0] - 46 + Math.sin(t * 4.3 + 1) * 10 * k, L[1] + 70 - Math.abs(Math.sin(t * 2.7 + 2)) * 40 * k];
    out.bl = -1; out.br = 1; out.tilt = Math.sin(t * 2.3) * 2.5; out.lean = Math.sin(t * 3.4) * 1.2;
  } else if (pose === 'point') {
    out.r = [R[0] + 150, R[1] - 24 + Math.sin(t * 6) * 2]; out.br = 1; out.point = true; out.lean = 2.5; out.tilt = -3;
  } else if (pose === 'facepalm') {
    out.dy = 36; out.shrug = 18; out.r = [spec.headC[0] - 6, spec.headC[1] + out.dy - 2]; out.br = 1; out.tilt = 9; out.lean = 2; out.squint = true;
  }
  if (walk !== undefined) {
    // arms swing against the stride, body leans into it
    const sw = Math.sin(walk);
    out.l = [out.l[0] + sw * 16, out.l[1] - Math.abs(sw) * 6];
    out.r = [out.r[0] - sw * 16, out.r[1] - Math.abs(sw) * 6];
    out.lean += 3.2; out.tilt += sw * 1.6;
  }
  return out;
}

// ------------------------------------------------------------------ face
type Side = {dy: number; rot: number};
type Mouth = {
  w: number; lift: number; lift2?: number; top: number; bot: number;
  teeth?: boolean; tongue?: boolean; fv?: boolean; ell?: Pt; wavy?: boolean; heavy?: boolean;
};
type Expr = {
  lid: number; tilt: number; open: number; bl: Side; br: Side; pupil: number;
  mouth: Mouth; cheeks: number; arcs?: boolean; fx?: string[]; look?: Pt;
};
const EX: Record<Expression, Expr> = {
  neutral: {lid: 0.05, tilt: 0, open: 1, bl: {dy: 0, rot: 0}, br: {dy: 0, rot: 0}, pupil: 1, cheeks: 0,
    mouth: {w: 22, lift: 4, top: 9, bot: 9}},
  happy: {lid: 0.0, tilt: 0, open: 0.92, bl: {dy: -7, rot: -4}, br: {dy: -7, rot: 4}, pupil: 1.15, cheeks: 0.8, fx: ['spark'],
    mouth: {w: 28, lift: 8, top: 4, bot: 40, teeth: true, tongue: true}},
  smug: {lid: 0.42, tilt: 0, open: 0.9, bl: {dy: 2, rot: 6}, br: {dy: -9, rot: -10}, pupil: 0.95, cheeks: 0.2, look: [0.5, 0],
    mouth: {w: 24, lift: 1, lift2: 13, top: 7, bot: 7, heavy: true}},
  shocked: {lid: 0, tilt: 0, open: 1.3, bl: {dy: -17, rot: -5}, br: {dy: -17, rot: 5}, pupil: 0.55, cheeks: 0, fx: ['sweat'],
    mouth: {w: 0, lift: 0, top: 0, bot: 0, ell: [11, 17]}},
  angry: {lid: 0.15, tilt: 17, open: 0.88, bl: {dy: 4, rot: 17}, br: {dy: 4, rot: -17}, pupil: 0.8, cheeks: 0, fx: ['vein'],
    mouth: {w: 24, lift: -6, top: -9, bot: 15, teeth: true}},
  sad: {lid: 0.2, tilt: -14, open: 0.95, bl: {dy: -2, rot: -16}, br: {dy: -2, rot: 16}, pupil: 1.2, cheeks: 0, fx: ['tear'], look: [0, 0.4],
    mouth: {w: 16, lift: -7, top: -9, bot: -9}},
  confused: {lid: 0.1, tilt: 0, open: 1, bl: {dy: -12, rot: -10}, br: {dy: 3, rot: 10}, pupil: 0.9, cheeks: 0, fx: ['question'], look: [-0.4, -0.5],
    mouth: {w: 20, lift: 0, top: 0, bot: 0, wavy: true}},
  laughing: {lid: 0, tilt: 0, open: 0, bl: {dy: -8, rot: -4}, br: {dy: -8, rot: 4}, pupil: 1, cheeks: 1, arcs: true, fx: ['joy'],
    mouth: {w: 32, lift: 10, top: 6, bot: 58, teeth: true, tongue: true}},
};
const VIS: Record<Exclude<Viseme, 'rest'>, Mouth> = {
  A: {w: 21, lift: 0, top: -14, bot: 46, teeth: true, tongue: true},
  E: {w: 30, lift: 2, top: -6, bot: 24, teeth: true},
  O: {w: 0, lift: 0, top: 0, bot: 0, ell: [13, 18]},
  U: {w: 0, lift: 0, top: 0, bot: 0, ell: [8, 10]},
  M: {w: 19, lift: 0, top: 3, bot: 3, heavy: true},
  F: {w: 23, lift: 0, top: -6, bot: 17, teeth: true, fv: true},
};

function MouthArt({m, f, uid, k}: {m: Mouth; f: FaceSpec; uid: string; k: number}) {
  const s = k;
  if (m.ell) {
    return <g>
      <ellipse rx={m.ell[0] * s} ry={m.ell[1] * s} fill="#3a1520" stroke={f.lip} strokeWidth={4} />
      <ellipse cy={m.ell[1] * s * 0.45} rx={m.ell[0] * s * 0.6} ry={m.ell[1] * s * 0.35} fill="#d9606f" />
    </g>;
  }
  if (m.wavy) {
    const w = m.w * s;
    return <path d={`M${-w},2 q${w / 4},-9 ${w / 2},0 t${w / 2},0 t${w / 2},0 t${w / 2},0`} fill="none" stroke={f.lip} strokeWidth={5} strokeLinecap="round" />;
  }
  const w = m.w * s, l1 = m.lift * s, l2 = (m.lift2 ?? m.lift) * s;
  const cl = -l1, cr = -l2;
  if (Math.abs(m.top - m.bot) < 3) {
    const cy = m.top * s;
    return <path d={`M${-w},${cl} Q0,${cy + (cl + cr) / 2 * 0.3} ${w},${cr}`} fill="none" stroke={f.lip} strokeWidth={m.heavy ? 6.5 : 5} strokeLinecap="round" />;
  }
  const d = `M${-w},${cl} Q0,${m.top * s} ${w},${cr} Q0,${m.bot * s} ${-w},${cl} Z`;
  const bm = (m.bot * s) / 2 + (cl + cr) / 4;
  return <g>
    <clipPath id={`${uid}mc`}><path d={d} /></clipPath>
    <path d={d} fill="#3a1520" />
    <g clipPath={`url(#${uid}mc)`}>
      {m.tongue && <ellipse cy={bm + 4} rx={w * 0.62} ry={Math.abs(m.bot * s) * 0.3} fill="#e0707e" />}
      {m.teeth && <rect x={-w} y={-30 * s} width={w * 2} height={30 * s + (m.fv ? 13 : 9) * s * 0.9} fill="#fffaf2" />}
      {m.fv && <path d={`M${-w},${bm + 8} Q0,${bm - 6} ${w},${bm + 8} L${w},40 L${-w},40Z`} fill="#e58a8a" />}
    </g>
    <path d={d} fill="none" stroke={f.lip} strokeWidth={4.5} strokeLinejoin="round" />
  </g>;
}

function Fx({kind, f, t}: {kind: string; f: FaceSpec; t: number}) {
  const [x, y] = f.fx;
  if (kind === 'sweat') {
    const dy = Math.sin(t * 4) * 2;
    return <path transform={`translate(${x},${y + dy})`} d="M0,-22 C10,-6 14,0 14,8 A14,14 0 1 1 -14,8 C-14,0 -10,-6 0,-22Z" fill="#9fd8ff" stroke="#3b7fb5" strokeWidth={3.5} />;
  }
  if (kind === 'vein') {
    const pulse = 1 + Math.sin(t * 9) * 0.06;
    return <g transform={`translate(${x - 4},${y - 10}) scale(${pulse})`} stroke="#e23c3c" strokeWidth={6} strokeLinecap="round" fill="none">
      <path d="M-16,-12 Q-6,-6 -4,-18 M16,-12 Q6,-6 4,-18 M-16,12 Q-6,6 -4,18 M16,12 Q6,6 4,18" />
    </g>;
  }
  if (kind === 'tear') {
    const fall = (t * 24) % 40;
    return <g>
      <path transform={`translate(${-f.eyeDx - 4},${f.eyeY + f.eyeR + 10 + fall * 0.5})`} opacity={1 - fall / 55} d="M0,-14 C7,-3 10,2 10,7 A10,10 0 1 1 -10,7 C-10,2 -7,-3 0,-14Z" fill="#a9defc" stroke="#3b7fb5" strokeWidth={3} />
    </g>;
  }
  if (kind === 'question') {
    const wob = Math.sin(t * 3) * 4;
    return <g transform={`translate(${x},${y - 6}) rotate(${12 + wob})`} stroke="#ffd447" strokeWidth={11} strokeLinecap="round" fill="none">
      <path d="M-14,-14 Q-14,-34 2,-34 Q18,-34 18,-18 Q18,-6 4,0 L4,10" stroke="#3a2a00" strokeWidth={19} />
      <path d="M-14,-14 Q-14,-34 2,-34 Q18,-34 18,-18 Q18,-6 4,0 L4,10" />
      <circle cx={4} cy={27} r={2} stroke="#3a2a00" strokeWidth={17} /><circle cx={4} cy={27} r={2} />
    </g>;
  }
  if (kind === 'spark') {
    const s = 1 + Math.sin(t * 6) * 0.18;
    return <path transform={`translate(${x},${y - 6}) scale(${s})`} d="M0,-20 Q3,-3 20,0 Q3,3 0,20 Q-3,3 -20,0 Q-3,-3 0,-20Z" fill="#fff6b0" stroke="#e0b32a" strokeWidth={3.5} />;
  }
  if (kind === 'joy') {
    const fall = (t * 30) % 30;
    return <g>
      {[-1, 1].map((sd) => (
        <path key={sd} transform={`translate(${sd * (f.eyeDx + 26)},${f.eyeY + 4 + fall * 0.4}) rotate(${sd * 55})`} opacity={1 - fall / 40} d="M0,-12 C6,-3 9,2 9,6 A9,9 0 1 1 -9,6 C-9,2 -6,-3 0,-12Z" fill="#a9defc" stroke="#3b7fb5" strokeWidth={3} />
      ))}
    </g>;
  }
  return null;
}

function Eye({f, side, ex, blink, look, c, lidColor}: {f: FaceSpec; side: -1 | 1; ex: Expr; blink: number; look: Pt; c: Ctx; lidColor: string}) {
  const x = side * f.eyeDx, y = f.eyeY, r = f.eyeR;
  const lens = f.style === 'shades';
  const open = Math.max(0.04, ex.open * blink);
  const ry = r * (f.style === 'wide' ? 1.0 : 1.18) * open;
  const idn = `${c.uid}e${side}`;
  const iris = f.iris[side === -1 ? 0 : 1];
  const closedArc = ex.arcs || (open <= 0.12);
  const sq = c.uid;
  if (closedArc && !lens) {
    const curve = ex.arcs ? -r * 1.1 : r * 0.7;
    return <path d={`M${x - r},${y + 2} Q${x},${y + curve + 2} ${x + r},${y + 2}`} fill="none" stroke={c.p.outline} strokeWidth={r * 0.34} strokeLinecap="round" />;
  }
  const tilt = side * ex.tilt; // + = inner side down for left eye
  const lidRect = (
    <g transform={`rotate(${tilt} ${x} ${y})`}>
      <rect x={x - r * 2} y={y - ry - r * 2.5} width={r * 4} height={r * 2.5 + ry * 2 * ex.lid} fill={lidColor} />
      <line x1={x - r * 2} x2={x + r * 2} y1={y - ry + ry * 2 * ex.lid} y2={y - ry + ry * 2 * ex.lid} stroke={c.p.outline} strokeWidth={4} />
    </g>
  );
  const px = x + (look[0] + ex_look(ex)[0]) * r * 0.34, py = y + (look[1] + ex_look(ex)[1]) * r * 0.3;
  const pr = r * 0.66 * ex.pupil;
  return (
    <g>
      <clipPath id={`${idn}`}><ellipse cx={x} cy={y} rx={r} ry={ry} /></clipPath>
      <ellipse cx={x} cy={y} rx={r + 3.5} ry={ry + 3.5} fill={c.p.outline} />
      <ellipse cx={x} cy={y} rx={r} ry={ry} fill={lens ? '#f4f0ff' : '#fffdf8'} />
      <g clipPath={`url(#${idn})`}>
        <circle cx={px} cy={py} r={pr} fill={iris} />
        <circle cx={px} cy={py} r={pr * 0.52} fill="#14101a" />
        <circle cx={px - pr * 0.3} cy={py - pr * 0.34} r={pr * 0.3} fill="#fff" />
        <circle cx={px + pr * 0.32} cy={py + pr * 0.38} r={pr * 0.13} fill="#fff" opacity={0.85} />
        {(ex.lid > 0.03 || ex.tilt !== 0) && lidRect}
      </g>
      <ellipse cx={x} cy={y} rx={r} ry={ry} fill="none" stroke={c.p.outline} strokeWidth={3.5} />
      {sq && null}
    </g>
  );
}
const ex_look = (e: Expr): Pt => e.look || [0, 0];

function Face({f, c, expression, viseme, blink, look, squint}: {f: FaceSpec; c: Ctx; expression: Expression; viseme: Viseme; blink: number; look: number; squint: boolean}) {
  const ex = EX[expression];
  const eb = squint ? 0.14 : blink;
  const vis = viseme === 'rest' ? null : VIS[viseme];
  const open = Math.min(1.15, 0.55 + 0.6 * c.energy); // louder = wider mouth
  const scaled = vis && {...vis, top: vis.top * open, bot: vis.bot * open, ell: vis.ell && ([vis.ell[0], vis.ell[1] * open] as Pt)};
  const base = viseme === 'rest' ? ex.mouth : {...scaled!, lift: VIS[viseme].lift + ex.mouth.lift * 0.45, lift2: (VIS[viseme].lift) + (ex.mouth.lift2 ?? ex.mouth.lift) * 0.45};
  const mouthK = f.mouthW / 30;
  const lens = f.style === 'shades';
  const ang = (b: Side, side: number) => `rotate(${b.rot * side * -1} ${side * f.eyeDx} ${f.eyeY - f.browGap + b.dy})`;
  const by = f.eyeY - f.browGap;
  return (
    <g>
      {ex.cheeks > 0 && [-1, 1].map((s) => (
        <ellipse key={s} cx={s * (f.eyeDx + 8)} cy={f.eyeY + f.eyeR * 1.9} rx={f.eyeR * 0.85} ry={f.eyeR * 0.5} fill={f.cheek} opacity={0.5 * ex.cheeks} />
      ))}
      {[-1, 1].map((s) => (
        <Eye key={s} f={f} side={s as -1 | 1} ex={ex} blink={eb} look={[look, 0]} c={c} lidColor={f.lidColor} />
      ))}
      {lens && [-1, 1].map((s) => {
        const tl = s * ex.tilt * 0.5 * -1 + (expression === 'smug' ? s * 3 : 0);
        return <g key={s} transform={`rotate(${tl} ${s * f.eyeDx} ${f.eyeY})`}>
          <rect x={s * f.eyeDx - f.eyeR * 1.35} y={f.eyeY - f.eyeR * 1.1} width={f.eyeR * 2.7} height={f.eyeR * (squint ? 1.2 : 2.1)} rx={f.eyeR * 0.7} fill="#0b0b10" fillOpacity={0.82} stroke="#2c2f3a" strokeWidth={5} />
          <path d={`M${s * f.eyeDx - f.eyeR},${f.eyeY - f.eyeR * 0.7} L${s * f.eyeDx - f.eyeR * 0.2},${f.eyeY - f.eyeR * 0.95}`} stroke="#fff" strokeWidth={4} strokeLinecap="round" opacity={0.5} />
        </g>;
      })}
      <path d={`M${-f.eyeDx + f.eyeR * 1.3},${f.eyeY - 2} L${f.eyeDx - f.eyeR * 1.3},${f.eyeY - 2}`} stroke={lens ? '#2c2f3a' : 'none'} strokeWidth={6} />
      {(['bl', 'br'] as const).map((k, i) => {
        const s = i === 0 ? -1 : 1, b = ex[k];
        const bw = f.eyeR * 1.35;
        return <g key={k} transform={ang(b, s)}>
          <path d={`M${s * f.eyeDx - bw},${by + b.dy + 2} Q${s * f.eyeDx},${by + b.dy - 7} ${s * f.eyeDx + bw},${by + b.dy + 2}`} fill="none" stroke={f.browColor} strokeWidth={f.browW} strokeLinecap="round" />
        </g>;
      })}
      <g transform={`translate(0,${f.mouthY})`}><MouthArt m={base} f={f} uid={c.uid} k={mouthK} /></g>
      {ex.fx?.map((k) => <Fx key={k} kind={k} f={f} t={c.t} />)}
    </g>
  );
}

// ------------------------------------------------------------- RigFrame
const blinkAt = (t: number, seed: number) => {
  const period = 3.1 + (seed % 5) * 0.37;
  const ph = (t + seed * 0.71) % period;
  if (ph > 0.18) return 1;
  const k = ph / 0.18;
  return Math.max(0.05, Math.abs(k * 2 - 1));
};

export function RigFrame({spec, ...p}: RigProps & {spec: RigSpec}) {
  const uid = 'r' + useId().replace(/[^a-zA-Z0-9]/g, '');
  const t = p.frame / p.fps;
  const energy = p.energy ?? 0;
  const pal = resolvePalette(spec.palette, p.palette);
  const acc = p.accessory && (spec.accessories.includes(p.accessory) || p.accessory === 'none') ? p.accessory : spec.accessory;
  const c: Ctx = {p: pal, acc, t, energy, uid, url: (n) => `url(#${uid}${n})`};
  const walking = p.walk !== undefined;
  const stride = walking ? Math.sin(p.walk!) : 0;
  const pose = handTargets(p.pose ?? 'idle', t, energy, spec, p.walk);
  const seed = p.seed ?? 1;
  const breath = Math.sin((t * Math.PI * 2) / 3.4 + seed);
  const sy = 1 + breath * 0.012, sx = 1 - breath * 0.005;
  const bounce = walking ? -Math.abs(stride) * 11 : -Math.pow(Math.sin(t * 2.2 + seed), 2) * 2.5 - energy * Math.abs(Math.sin(t * 9)) * 3;
  const blink = blinkAt(t, seed);
  const headDy = pose.dy + breath * -1.5 + energy * Math.sin(t * 11) * 1.4;
  const tilt = pose.tilt + Math.sin(t * 1.3 + seed) * 0.8 + energy * Math.sin(t * 7) * 1.2;
  const [nx, ny] = spec.neck;
  const [hx, hy] = spec.headC;
  const squint = (p.pose ?? 'idle') === 'facepalm';
  return (
    <svg viewBox="0 0 400 600" width={p.width ?? 400} height={(p.width ?? 400) * 1.5} style={{overflow: 'visible'}}>
      <defs>{spec.defs(c)}</defs>
      <g transform={p.flip ? 'translate(400,0) scale(-1,1)' : undefined}>
        {p.shadow !== false && <ellipse cx={200} cy={568} rx={118} ry={14} fill="#000" opacity={0.28} />}
        <g transform={`translate(0,${bounce}) translate(200,565) scale(${sx},${sy}) translate(-200,-565) rotate(${pose.lean} 200 565)`}>
          <g transform={walking ? `translate(200,486) skewX(${stride * 9}) translate(-200,-486)` : undefined}>{spec.legs(c)}</g>
          {spec.body(c)}
          <Arm c={c} spec={spec} s={spec.shoulderL} target={pose.l} bend={pose.bl} />
          <g transform={`translate(0,${headDy}) rotate(${tilt} ${nx} ${ny})`}>
            <g transform={`translate(${hx},${hy})`}>
              {spec.head(c)}
              <Face f={spec.face} c={c} expression={p.expression ?? 'neutral'} viseme={p.viseme ?? 'rest'} blink={blink} look={p.look ?? 0} squint={squint} />
              {spec.front(c)}
            </g>
          </g>
          <Arm c={c} spec={spec} s={[spec.shoulderR[0], spec.shoulderR[1] - pose.shrug]} target={pose.r} bend={pose.br} point={pose.point} />
          {spec.front2?.(c)}
        </g>
      </g>
    </svg>
  );
}
