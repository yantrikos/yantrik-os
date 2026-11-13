// GEMINI: bright azure twin-bun character with a flared tunic and big
// headphones. Two-toned eyes, endless enthusiasm, slightly too many ideas.
import React from 'react';
import {RigFrame, RigProps, RigSpec, lin, rad, lighten, darken} from './parts';

const spec: RigSpec = {
  name: 'GEMINI',
  palette: {main: '#4a90f0', accent: '#ffd35c', cloth: '#7a5cf0', hair: '#1f3a8f', outline: '#14275e', skin: '#8fc2ff'},
  accessories: ['headphones', 'cape'],
  accessory: 'headphones',
  shoulderL: [150, 318], shoulderR: [250, 318], headC: [200, 184], neck: [200, 280],
  arm: {l1: 52, l2: 58, w: 21, hand: 16, sleeve: true, sleeveLen: 0.45},
  face: {eyeDx: 42, eyeY: 4, eyeR: 24, style: 'wide', iris: ['#2563eb', '#8b3dff'], lidColor: '#8fc2ff',
    browColor: '#14275e', browW: 8, mouthY: 54, mouthW: 30, lip: '#14275e', cheek: '#ff9bc8', fx: [90, -66], browGap: 42},
  defs: (c) => <>
    {rad(c.uid + 'h', lighten(c.p.skin, 0.4), c.p.skin, darken(c.p.skin, 0.22))}
    {lin(c.uid + 'j', lighten(c.p.main, 0.28), c.p.main, darken(c.p.cloth, 0.1))}
    {rad(c.uid + 'k', lighten(c.p.hair, 0.4), c.p.hair, darken(c.p.hair, 0.3))}
    {lin(c.uid + 'g', lighten(c.p.accent, 0.4), c.p.accent, darken(c.p.accent, 0.25), true)}
    {lin(c.uid + 'v', lighten(c.p.cloth, 0.25), c.p.cloth, darken(c.p.cloth, 0.3), true)}
  </>,
  legs: (c) => <g stroke={c.p.outline} strokeWidth={5} strokeLinejoin="round">
    {[166, 234].map((x) => <g key={x}>
      <rect x={x - 14} y={478} width={28} height={68} rx={10} fill={c.url('h')} />
      <path d={`M${x - 24},548 Q${x - 26},568 ${x},568 Q${x + 26},568 ${x + 24},548 Q${x},538 ${x - 24},548Z`} fill="#fff" />
      <path d={`M${x - 20},556 Q${x},562 ${x + 20},556`} stroke={c.p.accent} strokeWidth={5} fill="none" />
    </g>)}
  </g>,
  body: (c) => <g>
    {c.acc === 'cape' && <path d="M148,290 Q90,420 78,520 Q200,550 322,520 Q310,420 252,290Z" fill={c.url('v')} stroke={c.p.outline} strokeWidth={6} strokeLinejoin="round" />}
    <path d="M154,288 Q200,276 246,288 Q262,330 276,418 Q290,480 300,508 Q200,538 100,508 Q110,480 124,418 Q138,330 154,288Z" fill={c.url('j')}
      stroke={c.p.outline} strokeWidth={6} strokeLinejoin="round" />
    <path d="M118,486 Q200,520 282,486" stroke={c.p.accent} strokeWidth={9} fill="none" strokeLinecap="round" />
    <path d="M160,300 Q150,380 128,470" stroke="#fff" strokeWidth={7} opacity={0.22} fill="none" strokeLinecap="round" />
    <g stroke={c.p.outline} strokeWidth={4}>
      <circle cx={186} cy={380} r={19} fill={c.url('g')} /><circle cx={214} cy={380} r={19} fill={c.url('g')} fillOpacity={0.8} />
      <circle cx={186} cy={380} r={7} fill="#fff" stroke="none" opacity={0.6} /><circle cx={214} cy={380} r={7} fill="#fff" stroke="none" opacity={0.6} />
    </g>
    {c.acc === 'headphones' && <g stroke={c.p.outline} strokeWidth={5} strokeLinejoin="round">
      <path d="M156,286 Q150,330 190,326 Q200,336 210,326 Q250,330 244,286 Q230,300 200,306 Q170,300 156,286Z" fill={c.url('g')} />
    </g>}
  </g>,
  head: (c) => <g stroke={c.p.outline} strokeWidth={6} strokeLinejoin="round">
    {[-1, 1].map((s) => <g key={s}>
      <circle cx={s * 74} cy={-104} r={44} fill={c.url('k')} />
      <circle cx={s * 74 - 12} cy={-116} r={13} fill="#fff" opacity={0.3} stroke="none" />
      <path d={`M${s * 74 - 30},-78 Q${s * 74},-62 ${s * 74 + 30},-78`} stroke={c.p.accent} strokeWidth={9} fill="none" />
    </g>)}
    {[-1, 1].map((s) => <ellipse key={s} cx={s * 102} cy={12} rx={14} ry={22} fill={c.url('h')} />)}
    <ellipse cx={0} cy={4} rx={108} ry={98} fill={c.url('h')} />
    <path d="M-106,-22 C-100,-100 100,-100 106,-22 C70,-70 -70,-70 -106,-22Z" fill={c.url('k')} />
    <ellipse cx={-42} cy={-56} rx={36} ry={12} fill="#fff" opacity={0.25} stroke="none" transform="rotate(-18 -42 -56)" />
  </g>,
  front: (c) => <g>
    {c.acc === 'headphones' && <g stroke={c.p.outline} strokeWidth={6} strokeLinejoin="round">
      <path d="M-104,0 C-116,-130 116,-130 104,0" fill="none" stroke={c.p.outline} strokeWidth={17} />
      <path d="M-104,0 C-116,-130 116,-130 104,0" fill="none" stroke={c.p.accent} strokeWidth={8} />
      {[-1, 1].map((s) => <g key={s}><rect x={s * 106 - 17} y={-26} width={34} height={58} rx={16} fill={c.url('v')} /><rect x={s * 106 - 7} y={-14}
        width={10} height={30} rx={5} fill="#fff" opacity={0.3} stroke="none" /></g>)}
    </g>}
  </g>,
  front2: (c) => c.acc === 'cape' ? <g stroke={c.p.outline} strokeWidth={5}><circle cx={200} cy={292} r={11} fill={c.url('g')} /></g> : null,
};

export const Gemini = (p: RigProps) => <RigFrame spec={spec} {...p} />;
export const geminiSpec = spec;
