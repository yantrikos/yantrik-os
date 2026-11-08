// Rooftop props: the radio mast with its beacon, the neon sign on the far
// building and the string of party lights.
import React from 'react';
import {beacon, Pulse, fitSize, FONT, hash} from './common';
import type {PropArt} from './state';

// --------------------------------------------------------------- antenna
export const Antenna: PropArt = ({frame, state, since}) => {
  const lit = beacon(state, frame);
  const legs = [0, 1, 2, 3, 4, 5, 6, 7];
  return (
    <g transform="translate(1700,0)">
      <defs>
        <radialGradient id="antGlow" cx="0.5" cy="0.5" r="0.5">
          <stop offset="0" stopColor="#ff3b30" stopOpacity="0.9" /><stop offset="1" stopColor="#ff3b30" stopOpacity="0" />
        </radialGradient>
      </defs>
      {/* lattice mast, wider at the foot */}
      <path d="M-26,716 L-5,300 M26,716 L5,300" stroke="#0e0b22" strokeWidth={9} strokeLinecap="round" />
      {legs.map((i) => {
        const y0 = 700 - i * 52, y1 = y0 - 52, w0 = 24 - i * 2.4, w1 = 24 - (i + 1) * 2.4;
        return <path key={i} d={`M${-w0},${y0} L${w1},${y1} M${w0},${y0} L${-w1},${y1} M${-w0},${y0} L${w0},${y0}`} stroke="#0e0b22" strokeWidth={4} fill="none" />;
      })}
      <path d="M-20,520 L-62,498 M20,470 L70,452" stroke="#0e0b22" strokeWidth={6} strokeLinecap="round" />
      <ellipse cx={-70} cy={494} rx={14} ry={22} fill="#241d4a" stroke="#0e0b22" strokeWidth={4} transform="rotate(-20 -70 494)" />
      <rect x={-34} y={716} width={68} height={18} rx={4} fill="#3a3f55" stroke="#0e0b22" strokeWidth={4} />
      {/* beacon */}
      <circle cx={0} cy={292} r={90} fill="url(#antGlow)" opacity={lit * 0.9} />
      <circle cx={0} cy={292} r={13} fill={lit > 0.5 ? '#ff4a3f' : '#5a1a1c'} stroke="#0e0b22" strokeWidth={4} />
      <circle cx={-4} cy={288} r={4} fill="#fff" opacity={0.3 + 0.6 * lit} />
      <g transform="translate(0,292)"><Pulse since={since} frame={frame} r0={14} r1={110} color="#ff6a5e" /></g>
    </g>
  );
};

// -------------------------------------------------------------- neon sign
export const NeonSign: PropArt = ({frame, state, text}) => {
  const shown = (text || 'OPEN LATE').toUpperCase();
  const fs = fitSize(shown, 340, 76, 28);
  const flick = state === 'flicker' ? (hash(Math.floor(frame / 3)) > 0.35 ? 1 : 0.15) : state === 'off' ? 0 : 1;
  const col = flick > 0.5 ? '#ff5fe0' : '#5a2a58';
  return (
    <g transform="translate(880,0)">
      <defs>
        <filter id="neonBlur" x="-30%" y="-60%" width="160%" height="220%"><feGaussianBlur stdDeviation="9" /></filter>
      </defs>
      <rect x={60} y={420} width={10} height={300} fill="#0e0b22" /><rect x={350} y={420} width={10} height={300} fill="#0e0b22" />
      <rect x={0} y={270} width={420} height={150} rx={14} fill="#120d2c" stroke="#0a0720" strokeWidth={6} />
      <rect x={14} y={284} width={392} height={122} rx={8} fill="none" stroke={col} strokeWidth={4} opacity={0.55 + 0.45 * flick} />
      {flick > 0.5 && <text x={210} y={345 + fs * 0.35} textAnchor="middle" fontFamily={FONT} fontWeight={700} fontSize={fs} fill="#ff5fe0" filter="url(#neonBlur)" opacity={0.85}>{shown}</text>}
      <text x={210} y={345 + fs * 0.35} textAnchor="middle" fontFamily={FONT} fontWeight={700} fontSize={fs} fill={flick > 0.5 ? '#ffe4f8' : '#4a2a48'} stroke={col} strokeWidth={flick > 0.5 ? 2 : 1}>{shown}</text>
    </g>
  );
};

// ----------------------------------------------------------- string lights
export const StringLights: PropArt = ({frame, state}) => (
  <g>
    <path d="M0,150 Q480,260 960,150 T1920,150" fill="none" stroke="#1a1d2c" strokeWidth={4} />
    {Array.from({length: 17}).map((_, k) => {
      const t = k / 16, px = t * 1920;
      const y = 150 + 0.5 * 110 * Math.sin(t * Math.PI * 4 - Math.PI / 2) * -1 + 55;
      const lit = state === 'off' ? 0 : state === 'flicker' ? (hash(Math.floor(frame / 4) * 17 + k) > 0.4 ? 1 : 0.1) : 0.7 + 0.3 * Math.sin(frame * 0.1 + k);
      return (
        <g key={k}>
          <circle cx={px} cy={y - 28} r={34} fill="#ffd98a" opacity={0.12 * lit} />
          <circle cx={px} cy={y - 28} r={9} fill={lit > 0.05 ? '#fff1b8' : '#5a5448'} opacity={lit > 0.05 ? lit : 1} />
        </g>
      );
    })}
  </g>
);
