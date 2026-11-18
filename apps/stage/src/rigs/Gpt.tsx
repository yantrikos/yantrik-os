// GPT: tall, lanky, emerald salesman with a towering quiff and a tie.
// Radiates confidence he has not earned.
import React from 'react';
import {RigFrame, RigProps, RigSpec, lin, rad, lighten, darken} from './parts';

const spec: RigSpec = {
  name: 'GPT',
  palette: {main: '#2fb58a', accent: '#f4fff9', cloth: '#1f7f66', hair: '#0f4a3b', outline: '#0b3328', skin: '#7fd9b5'},
  accessories: ['tie', 'bowtie'],
  accessory: 'tie',
  shoulderL: [146, 322], shoulderR: [254, 322], headC: [200, 172], neck: [200, 268],
  arm: {l1: 62, l2: 66, w: 20, hand: 15, sleeve: true, sleeveLen: 0.35},
  face: {eyeDx: 38, eyeY: 4, eyeR: 19, style: 'round', iris: ['#0f5a46', '#0f5a46'], lidColor: '#7fd9b5',
    browColor: '#0b3328', browW: 10, mouthY: 52, mouthW: 30, lip: '#0b3328', cheek: '#ff9bb0', fx: [84, -64], browGap: 36},
  defs: (c) => <>
    {rad(c.uid + 'h', lighten(c.p.skin, 0.35), c.p.skin, darken(c.p.skin, 0.25))}
    {lin(c.uid + 'j', lighten(c.p.cloth, 0.28), c.p.cloth, darken(c.p.cloth, 0.3))}
    {lin(c.uid + 's', '#ffffff', '#e8faf3', '#b5d8cb', true)}
    {lin(c.uid + 'k', lighten(c.p.hair, 0.35), c.p.hair, darken(c.p.hair, 0.3))}
    {lin(c.uid + 'p', '#34465a', '#232f3d', '#141c26', true)}
  </>,
  legs: (c) => <g stroke={c.p.outline} strokeWidth={5} strokeLinejoin="round">
    {[168, 232].map((x) => {
      const d = x > 200 ? 1 : -1;
      return <g key={x}>
        <path d={`M${x - 20},470 L${x - 17},548 L${x + 17},548 L${x + 20},470Z`} fill={c.url('p')} />
        <path d={`M${x - 24},546 Q${x - 26},566 ${x - 4},566 L${x + 40 * d},566 Q${x + 44 * d},548 ${x + 20 * d},546Z`} fill="#3b2a22" />
      </g>;
    })}
  </g>,
  body: (c) => <g>
    <path d="M140,278 Q200,262 260,278 Q276,300 272,350 L262,478 Q200,494 138,478 L128,350 Q124,300 140,278Z" fill={c.url('j')} stroke={c.p.outline} strokeWidth={6} strokeLinejoin="round" />
    <path d="M172,272 L228,272 L222,420 L178,420Z" fill={c.url('s')} stroke={c.p.outline} strokeWidth={4} strokeLinejoin="round" />
    <path d="M170,268 L200,330 L178,420 L146,300Z M230,268 L200,330 L222,420 L254,300Z" fill={darken(c.p.cloth, 0.08)} stroke={c.p.outline} strokeWidth={4} strokeLinejoin="round" />
    <path d="M150,300 Q146,380 150,470" stroke="#fff" strokeWidth={6} opacity={0.18} fill="none" strokeLinecap="round" />
    <rect x={226} y={376} width={20} height={5} rx={2} fill="#fff" opacity={0.4} />
    {c.acc === 'tie' && <g stroke={c.p.outline} strokeWidth={4} strokeLinejoin="round">
      <path d="M190,290 L210,290 L206,308 L194,308Z" fill="#ffcf4a" />
      <path d="M194,308 L206,308 L216,402 L200,420 L184,402Z" fill="#ffcf4a" />
      <path d="M200,330 L212,380" stroke="#fff" opacity={0.5} strokeWidth={5} />
    </g>}
    {c.acc === 'bowtie' && <g stroke={c.p.outline} strokeWidth={4} strokeLinejoin="round" fill="#ffcf4a">
      <path d="M200,288 L170,274 L170,302Z M200,288 L230,274 L230,302Z" /><circle cx={200} cy={288} r={8} />
    </g>}
  </g>,
  head: (c) => <g stroke={c.p.outline} strokeWidth={6} strokeLinejoin="round">
    {[-1, 1].map((s) => <ellipse key={s} cx={s * 94} cy={14} rx={16} ry={24} fill={c.url('h')} />)}
    <rect x={-96} y={-86} width={192} height={176} rx={62} fill={c.url('h')} />
    <ellipse cx={-44} cy={-52} rx={36} ry={14} fill="#fff" opacity={0.22} stroke="none" transform="rotate(-16 -44 -52)" />
    <path d="M-5,24 Q0,34 8,28" fill="none" stroke={c.p.outline} strokeWidth={4} strokeLinecap="round" opacity={0.6} />
  </g>,
  front: (c) => <g stroke={c.p.outline} strokeWidth={6} strokeLinejoin="round" fill={c.url('k')}>
    <path d="M-100,-30 C-110,-96 -76,-120 -28,-128 C-10,-190 80,-190 110,-130 C132,-96 110,-60 98,-30 C92,-62 86,-76 62,-86 C20,-70 -40,-70 -80,-72 C-92,-64 -96,-48 -100,-30Z" />
    <path d="M-30,-128 C0,-160 56,-166 84,-142" fill="none" stroke="#fff" opacity={0.3} strokeWidth={7} strokeLinecap="round" />
    <path d="M10,-100 C30,-120 64,-122 80,-108" fill="none" stroke={c.p.outline} opacity={0.5} strokeWidth={4} strokeLinecap="round" />
  </g>,
};

export const Gpt = (p: RigProps) => <RigFrame spec={spec} {...p} />;
export const gptSpec = spec;
