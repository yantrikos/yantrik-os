// GROK: near-black hooded smart-aleck with amber rim light, shades and
// hands deep in the hoodie pocket. Wide, slouched, unimpressed.
import React from 'react';
import {RigFrame, RigProps, RigSpec, lin, rad, lighten, darken} from './parts';

const RIM = '#ff9a4a';
const spec: RigSpec = {
  name: 'GROK',
  palette: {main: '#2a2d38', accent: '#ff9a4a', cloth: '#23252f', hair: '#16171d', outline: '#07070a', skin: '#596078'},
  accessories: ['shades', 'chain'],
  accessory: 'shades',
  shoulderL: [124, 338], shoulderR: [276, 338], headC: [200, 206], neck: [200, 300],
  arm: {l1: 54, l2: 56, w: 30, hand: 17, sleeve: true, sleeveLen: 0.8},
  face: {eyeDx: 42, eyeY: 4, eyeR: 20, style: 'shades', iris: ['#ffffff', '#ffffff'], lidColor: '#4a4f60',
    browColor: '#e9e9ee', browW: 8, mouthY: 52, mouthW: 30, lip: '#07070a', cheek: '#ff9a4a', fx: [96, -50], browGap: 31},
  restHand: (side) => (side === 'l' ? [168, 452] : [232, 452]),
  defs: (c) => <>
    {rad(c.uid + 'h', lighten(c.p.skin, 0.25), c.p.skin, darken(c.p.skin, 0.4))}
    {lin(c.uid + 'j', lighten(c.p.cloth, 0.3), c.p.cloth, darken(c.p.cloth, 0.5))}
    {lin(c.uid + 'q', lighten(c.p.cloth, 0.12), '#1a1c24', '#0b0b10', true)}
  </>,
  legs: (c) => <g stroke={c.p.outline} strokeWidth={5} strokeLinejoin="round">
    {[160, 242].map((x) => {
      const d = x > 200 ? 1 : -1;
      return <g key={x}>
        <path d={`M${x - 30},486 L${x - 34},548 L${x + 34},548 L${x + 30},486Z`} fill={c.url('q')} />
        <path d={`M${x - 40},548 Q${x - 44},572 ${x - 4},572 L${x + 44 * d},572 Q${x + 50 * d},548 ${x + 30 * d},544Z`} fill="#f0f0f4" />
        <path d={`M${x - 36},566 L${x + 40},566`} stroke={RIM} strokeWidth={6} />
      </g>;
    })}
  </g>,
  body: (c) => <g>
    <path d="M142,292 Q200,278 258,292 Q300,310 306,380 Q312,450 296,496 Q200,524 104,496 Q88,450 94,380 Q100,310 142,292Z" fill={c.url('j')}
      stroke={c.p.outline} strokeWidth={6} strokeLinejoin="round" />
    <path d="M296,330 Q312,430 296,496" stroke={RIM} strokeWidth={5} fill="none" opacity={0.85} strokeLinecap="round" />
    <path d="M104,340 Q92,420 104,490" stroke={RIM} strokeWidth={3} fill="none" opacity={0.4} strokeLinecap="round" />
    <path d="M128,430 Q200,452 272,430 L286,490 Q200,516 114,490Z" fill={c.url('q')} stroke={c.p.outline} strokeWidth={5} strokeLinejoin="round" />
    <path d="M132,434 Q200,456 268,434" stroke={RIM} strokeWidth={3} fill="none" opacity={0.6} />
    <path d="M182,312 L180,372 M218,312 L220,372" stroke="#d9d9e0" strokeWidth={4} strokeLinecap="round" />
    <circle cx={180} cy={376} r={5} fill="#d9d9e0" /><circle cx={220} cy={376} r={5} fill="#d9d9e0" />
    {c.acc === 'chain' && <path d="M158,312 Q200,386 242,312" stroke="#ffd35c" strokeWidth={6} fill="none" strokeLinecap="round" strokeDasharray="1 8" />}
  </g>,
  head: (c) => <g stroke={c.p.outline} strokeWidth={6} strokeLinejoin="round">
    <path d="M-124,10 C-134,-90 -64,-130 6,-150 C70,-128 134,-90 126,10 C130,70 90,110 0,114 C-90,110 -128,70 -124,10Z" fill={c.url('j')} />
    <path d="M126,-10 C132,-70 94,-110 40,-136" stroke={RIM} strokeWidth={5} fill="none" opacity={0.9} strokeLinecap="round" />
    <path d="M-120,-20 C-116,-70 -80,-100 -40,-122" stroke={RIM} strokeWidth={3} fill="none" opacity={0.4} strokeLinecap="round" />
    <ellipse cx={0} cy={14} rx={92} ry={88} fill={c.url('h')} />
    <ellipse cx={-30} cy={-46} rx={34} ry={11} fill="#fff" opacity={0.16} stroke="none" transform="rotate(-14 -30 -46)" />
  </g>,
  front: (c) => <g stroke={c.p.outline} strokeWidth={6} strokeLinejoin="round">
    <path d="M-98,-18 C-86,-76 -36,-92 6,-96 C50,-92 92,-72 100,-18 C70,-58 -60,-62 -98,-18Z" fill={c.url('j')} />
    <path d="M-80,-40 C-40,-70 40,-70 82,-40" stroke={RIM} strokeWidth={3} fill="none" opacity={0.5} strokeLinecap="round" />
  </g>,
};

export const Grok = (p: RigProps) => <RigFrame spec={spec} {...p} />;
export const grokSpec = spec;
