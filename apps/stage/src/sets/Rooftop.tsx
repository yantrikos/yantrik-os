// Set: the rooftop at night. Skyline, twinkling stars, string lights.
import React from 'react';
import type {PropViews} from '../props/state';
import {Antenna, NeonSign, StringLights} from '../props/RoofProps';

const rnd = (n: number) => { const x = Math.sin(n * 91.7 + 17.3) * 24634.6345; return x - Math.floor(x); };

export const Rooftop: React.FC<{frame: number; props: PropViews}> = ({frame, props}) => {
  const buildings = [];
  let x = -40, i = 0;
  while (x < 1960) {
    const w = 120 + rnd(i) * 150, h = 160 + rnd(i + 40) * 330;
    buildings.push({x, w, h, i});
    x += w - 6; i++;
  }
  return (
    <svg viewBox="0 0 1920 1080" width={1920} height={1080} style={{position: 'absolute', left: 0, top: 0}}>
      <defs>
        <linearGradient id="rtSky" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stopColor="#0b1030" /><stop offset="0.7" stopColor="#3b2a6e" /><stop offset="1" stopColor="#b5527a" /></linearGradient>
        <linearGradient id="rtFloor" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stopColor="#3a3f55" /><stop offset="1" stopColor="#1a1d2c" /></linearGradient>
      </defs>
      <rect width={1920} height={1080} fill="url(#rtSky)" />
      {Array.from({length: 90}).map((_, k) => <circle key={k} cx={rnd(k) * 1920} cy={rnd(k + 9) * 460} r={1 + rnd(k + 3) * 2} fill="#fff" opacity={0.35 + 0.65 * Math.abs(Math.sin(frame * 0.04 + k))} />)}
      <circle cx={1500} cy={170} r={70} fill="#fff6d8" /><circle cx={1476} cy={150} r={70} fill="#0b1030" opacity={0.0} />
      <circle cx={1500} cy={170} r={140} fill="#fff6d8" opacity={0.08} />
      {buildings.map(({x: bx, w, h, i: bi}) => (
        <g key={bi}>
          <rect x={bx} y={760 - h} width={w} height={h + 40} fill={bi % 2 ? '#1a1636' : '#221b45'} />
          {Array.from({length: Math.floor(h / 44)}).map((_, r) => Array.from({length: Math.floor(w / 36)}).map((__, c) => (rnd(bi * 100 + r * 10 + c) > 0.55
            ? <rect key={`${r}-${c}`} x={bx + 14 + c * 36} y={760 - h + 18 + r * 44} width={16} height={22} fill="#ffd98a" opacity={0.75} /> : null)))}
        </g>
      ))}
      <g transform="translate(300,330)"><rect x={0} y={90} width={90} height={80} fill="#2a2147" /><path d="M-10,90 L45,40 L100,90Z" fill="#33295a" /><rect x={20} y={170} width={8} height={300} fill="#2a2147" /><rect x={62} y={170} width={8} height={300} fill="#2a2147" /></g>
      {/* parapet, floor, string lights */}
      <rect x={0} y={720} width={1920} height={50} fill="#2c3048" /><rect x={0} y={714} width={1920} height={12} fill="#4a5070" />
      <rect x={0} y={770} width={1920} height={310} fill="url(#rtFloor)" />
      {Array.from({length: 12}).map((_, k) => <line key={k} x1={k * 180 + 30} x2={k * 180 - 40} y1={770} y2={1080} stroke="#fff" strokeOpacity={0.05} strokeWidth={2} />)}
      <NeonSign frame={frame} {...props['neon-sign']} />
      <Antenna frame={frame} {...props.antenna} />
      <StringLights frame={frame} {...props['string-lights']} />
    </svg>
  );
};
