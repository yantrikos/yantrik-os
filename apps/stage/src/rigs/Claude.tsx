// CLAUDE: a plump, earnest terracotta creature in a cream scarf. Pear
// silhouette, round ears, curly tuft, round glasses. Polite to a fault.
import React from 'react';
import {RigFrame, RigProps, RigSpec, lin, rad, lighten, darken} from './parts';

const spec: RigSpec = {
  name: 'CLAUDE',
  palette: {main: '#d9774f', accent: '#f6e6cc', cloth: '#f6e6cc', hair: '#8f3f26', outline: '#5a2616', skin: '#d9774f'},
  accessories: ['glasses', 'beret'],
  accessory: 'glasses',
  shoulderL: [132, 336], shoulderR: [268, 336], headC: [200, 196], neck: [200, 290],
  arm: {l1: 52, l2: 56, w: 27, hand: 17},
  face: {eyeDx: 44, eyeY: 6, eyeR: 22, style: 'round', iris: ['#6a3a1f', '#6a3a1f'], lidColor: '#d9774f',
    browColor: '#6e2f1b', browW: 8, mouthY: 52, mouthW: 30, lip: '#5a2616', cheek: '#ff8f7a', fx: [92, -66], browGap: 40},
  defs: (c) => <>
    {rad(c.uid + 'h', lighten(c.p.main, 0.28), c.p.main, darken(c.p.main, 0.28))}
    {lin(c.uid + 'b', lighten(c.p.main, 0.2), c.p.main, darken(c.p.main, 0.3))}
    {lin(c.uid + 'c', lighten(c.p.accent, 0.5), c.p.accent, darken(c.p.accent, 0.12), true)}
    {lin(c.uid + 'k', lighten(c.p.hair, 0.25), c.p.hair, darken(c.p.hair, 0.3))}
  </>,
  legs: (c) => <g stroke={c.p.outline} strokeWidth={5}>
    {[150, 250].map((x) => <g key={x}>
      <ellipse cx={x} cy={548} rx={42} ry={21} fill={c.url('b')} />
      <ellipse cx={x - 12} cy={541} rx={16} ry={6} fill="#fff" opacity={0.2} stroke="none" />
    </g>)}
  </g>,
  body: (c) => <g>
    <path d="M152,292 C118,322 84,420 100,512 Q200,566 300,512 C316,420 282,322 248,292 Z" fill={c.url('b')} stroke={c.p.outline} strokeWidth={6} strokeLinejoin="round" />
    <path d="M250,300 C284,340 302,430 292,510 Q272,530 250,536 C276,460 270,370 232,308Z" fill={darken(c.p.main, 0.35)} opacity={0.35} />
    <ellipse cx={200} cy={440} rx={64} ry={82} fill={c.url('c')} stroke={c.p.outline} strokeWidth={4} opacity={0.97} />
    <ellipse cx={176} cy={398} rx={22} ry={34} fill="#fff" opacity={0.3} transform="rotate(14 176 398)" />
    <path d="M140,284 Q200,330 260,284 L268,310 Q200,356 132,310Z" fill={c.url('c')} stroke={c.p.outline} strokeWidth={5} strokeLinejoin="round" />
    {[0, 1, 2, 3].map((i) => <path key={i} d={`M${158 + i * 22},${302 + Math.sin(i * 0.8) * 3} l6,26`} stroke={c.p.hair} strokeWidth={6} strokeLinecap="round" opacity={0.7} />)}
  </g>,
  head: (c) => <g stroke={c.p.outline} strokeWidth={6} strokeLinejoin="round">
    {[-1, 1].map((s) => <g key={s}>
      <circle cx={s * 100} cy={-22} r={32} fill={c.url('h')} />
      <circle cx={s * 100} cy={-20} r={17} fill={darken(c.p.main, 0.1)} stroke="none" opacity={0.7} />
    </g>)}
    <rect x={-112} y={-100} width={224} height={192} rx={92} fill={c.url('h')} />
    <ellipse cx={0} cy={40} rx={74} ry={50} fill={c.url('c')} stroke="none" opacity={0.95} />
    <ellipse cx={-48} cy={-62} rx={44} ry={20} fill="#fff" opacity={0.2} stroke="none" transform="rotate(-18 -48 -62)" />
    <g fill={c.url('k')} strokeWidth={5}>
      <path d="M-6,-98 C-30,-140 8,-158 20,-130 C30,-150 56,-128 34,-104 C20,-96 4,-94 -6,-98Z" />
      <path d="M-36,-94 C-60,-118 -34,-140 -16,-116" fill="none" stroke={c.p.hair} />
    </g>
  </g>,
  front: (c) => <g>
    {c.acc === 'glasses' && <g fill="#fff" fillOpacity={0.08} stroke="#c8943a" strokeWidth={7}>
      {[-1, 1].map((s) => <circle key={s} cx={s * 44} cy={6} r={33} />)}
      <path d="M-11,2 Q0,-6 11,2" fill="none" />
      <path d="M-34,-16 Q-26,-26 -14,-26" stroke="#fff" strokeWidth={4} fill="none" opacity={0.7} strokeLinecap="round" />
    </g>}
    {c.acc === 'beret' && <g stroke={c.p.outline} strokeWidth={5}>
      <path d="M-86,-76 C-90,-134 40,-150 90,-108 C104,-90 70,-70 -86,-76Z" fill="#7b2d3a" />
      <path d="M-70,-90 C-40,-128 20,-134 50,-124" fill="none" stroke="#fff" opacity={0.25} strokeWidth={6} strokeLinecap="round" />
      <circle cx={6} cy={-136} r={7} fill="#7b2d3a" />
    </g>}
  </g>,
  front2: (c) => <g>
    <path d="M232,318 Q252,350 244,402 Q232,410 220,402 Q226,356 214,330Z" fill={c.url('c')} stroke={c.p.outline} strokeWidth={5} strokeLinejoin="round" />
    <path d="M226,382 l14,-2 M228,392 l14,-2" stroke={c.p.hair} strokeWidth={5} opacity={0.7} />
  </g>,
};

export const Claude = (p: RigProps) => <RigFrame spec={spec} {...p} />;
export const claudeSpec = spec;
