// Set: the break room. Warm light, a window onto dusk, a vending machine
// with one lonely snack, a coffee machine that has seen things.
import React from 'react';
import type {PropViews} from '../props/state';
import {CoffeeMachine, VendingMachine, WallClock} from '../props/BreakProps';

export const BreakRoom: React.FC<{frame: number; props: PropViews}> = ({frame, props}) => {
  return (
    <svg viewBox="0 0 1920 1080" width={1920} height={1080} style={{position: 'absolute', left: 0, top: 0}}>
      <defs>
        <linearGradient id="brWall" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stopColor="#f6d9a8" /><stop offset="1" stopColor="#e9b676" /></linearGradient>
        <linearGradient id="brFloor" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stopColor="#a4693a" /><stop offset="1" stopColor="#6b3f21" /></linearGradient>
        <linearGradient id="brSky" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stopColor="#5b4fa8" /><stop offset="0.6" stopColor="#ff8a65" /><stop offset="1" stopColor="#ffcf8a" /></linearGradient>
        <linearGradient id="brVend" x1="0" y1="0" x2="1" y2="0"><stop offset="0" stopColor="#d6334b" /><stop offset="1" stopColor="#8f1d33" /></linearGradient>
        <radialGradient id="brLamp" cx="0.5" cy="0" r="0.9"><stop offset="0" stopColor="#fff2c4" stopOpacity="0.55" /><stop offset="1" stopColor="#fff2c4" stopOpacity="0" /></radialGradient>
      </defs>
      <rect width={1920} height={1080} fill="url(#brWall)" />
      {/* wainscot */}
      <rect x={0} y={560} width={1920} height={140} fill="#7fb7a4" /><rect x={0} y={552} width={1920} height={14} fill="#f3ead8" />
      {[...Array(24)].map((_, i) => <line key={i} x1={i * 84} x2={i * 84} y1={566} y2={700} stroke="#6aa08d" strokeWidth={3} />)}
      {/* window */}
      <g transform="translate(560,110)">
        <rect width={560} height={380} rx={10} fill="#3a2a22" />
        <rect x={16} y={16} width={528} height={348} fill="url(#brSky)" />
        <path d="M16,364 L16,290 L70,290 L70,240 L130,240 L130,300 L200,300 L200,210 L270,210 L270,320 L330,320 L330,260 L410,260 L410,330 L480,330 L480,280 L544,280 L544,364Z" fill="#2a2147" />
        {[[90, 262], [230, 236], [244, 270], [360, 282], [500, 306]].map(([x, y], i) => <rect key={i} x={x} y={y} width={12} height={14} fill="#ffd35c" opacity={0.5 + 0.5 * Math.sin(frame * 0.05 + i)} />)}
        <circle cx={420} cy={90} r={34} fill="#fff3d0" opacity={0.9} />
        <line x1={280} y1={16} x2={280} y2={364} stroke="#3a2a22" strokeWidth={10} /><line x1={16} y1={190} x2={544} y2={190} stroke="#3a2a22" strokeWidth={10} />
      </g>
      <WallClock frame={frame} {...props['wall-clock']} />
      {/* poster */}
      <g transform="translate(160,170)">
        <rect width={260} height={340} rx={6} fill="#fff8e8" stroke="#3a2a22" strokeWidth={7} />
        <rect x={24} y={28} width={212} height={150} fill="#ffd35c" />
        <path d="M24,178 L90,100 L130,150 L170,90 L236,178Z" fill="#e0912e" />
        <text x={130} y={236} textAnchor="middle" fontFamily="Fredoka, sans-serif" fontWeight={700} fontSize={26} fill="#3a2a22">HANG IN</text>
        <text x={130} y={270} textAnchor="middle" fontFamily="Fredoka, sans-serif" fontWeight={700} fontSize={26} fill="#3a2a22">THERE, PROD</text>
      </g>
      <VendingMachine frame={frame} {...props['vending-machine']} />
      {/* counter + coffee machine */}
      <g>
        <rect x={1120} y={500} width={420} height={210} rx={6} fill="#5a3a24" stroke="#2b1a10" strokeWidth={6} />
        <rect x={1100} y={486} width={460} height={28} rx={8} fill="#f3ead8" stroke="#2b1a10" strokeWidth={5} />
        <CoffeeMachine frame={frame} {...props['coffee-machine']} />
        <rect x={1180} y={548} width={190} height={14} rx={6} fill="#3a2a22" opacity={0.35} />
      </g>
      {/* lamp glows */}
      {[300, 960, 1560].map((x) => <g key={x}><rect x={x - 80} y={0} width={160} height={14} rx={7} fill="#fff6d8" /><ellipse cx={x} cy={160} rx={260} ry={300} fill="url(#brLamp)" /></g>)}
      {/* floor */}
      <rect x={0} y={700} width={1920} height={380} fill="url(#brFloor)" />
      {Array.from({length: 7}).map((_, i) => <line key={i} x1={0} x2={1920} y1={730 + i * i * 9 + i * 22} y2={730 + i * i * 9 + i * 22} stroke="#3e2412" strokeOpacity={0.5} strokeWidth={3} />)}
      {Array.from({length: 18}).map((_, i) => <line key={i} x1={(i * 130 + 40) % 1920} x2={(i * 130 + 40) % 1920 + ((i % 2) * 2 - 1) * 40} y1={702} y2={1080} stroke="#3e2412" strokeOpacity={0.22} strokeWidth={2} />)}
      <rect x={0} y={692} width={1920} height={14} fill="#f3ead8" />
    </svg>
  );
};
