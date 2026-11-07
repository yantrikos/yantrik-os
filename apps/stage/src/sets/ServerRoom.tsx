// Set: the server room. Cool teal light, rows of blinking racks, a wall
// dashboard that insists everything is fine, a spinning red alert lamp.
import React from 'react';

const rnd = (n: number) => { const x = Math.sin(n * 127.1 + 311.7) * 43758.5453; return x - Math.floor(x); };

function Rack({x, w, h, y, frame, seed}: {x: number; w: number; h: number; y: number; frame: number; seed: number}) {
  const units = Math.floor(h / 34);
  return (
    <g>
      <rect x={x} y={y} width={w} height={h} rx={8} fill="url(#rackG)" stroke="#05080f" strokeWidth={4} />
      <rect x={x + 8} y={y + 8} width={w - 16} height={h - 16} rx={4} fill="#0a1019" />
      {Array.from({length: units}).map((_, i) => {
        const uy = y + 14 + i * 34;
        return <g key={i}>
          <rect x={x + 14} y={uy} width={w - 28} height={28} rx={3} fill={i % 3 === 0 ? '#1b2736' : '#16202d'} stroke="#2d3d52" strokeWidth={1.5} />
          {Array.from({length: 5}).map((__, k) => {
            const on = rnd(seed * 100 + i * 7 + k + Math.floor((frame + seed * 13 + i * 5 + k * 9) / (14 + (k * 7) % 19))) > 0.45;
            const col = k === 4 ? '#ffb02e' : k % 2 ? '#3cff9d' : '#38b6ff';
            return <circle key={k} cx={x + 28 + k * 14} cy={uy + 14} r={3.5} fill={col} opacity={on ? 1 : 0.18} />;
          })}
          <rect x={x + w - 84} y={uy + 8} width={56} height={12} rx={2} fill="#0a1019" />
          {Array.from({length: 6}).map((__, k) => <rect key={k} x={x + w - 80 + k * 9} y={uy + 10} width={5} height={8} fill="#26374d" />)}
        </g>;
      })}
    </g>
  );
}

export const ServerRoom: React.FC<{frame: number}> = ({frame}) => {
  const racks = [];
  for (let i = 0; i < 9; i++) racks.push(<Rack key={i} x={-20 + i * 225} w={205} h={560} y={110} frame={frame} seed={i + 1} />);
  const spin = (frame * 5) % 360;
  const flash = 0.55 + 0.45 * Math.sin(frame * 0.35);
  return (
    <svg viewBox="0 0 1920 1080" width={1920} height={1080} style={{position: 'absolute', left: 0, top: 0}}>
      <defs>
        <linearGradient id="srWall" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stopColor="#0d1b2e" /><stop offset="1" stopColor="#12304a" /></linearGradient>
        <linearGradient id="rackG" x1="0" y1="0" x2="1" y2="0"><stop offset="0" stopColor="#2a3a50" /><stop offset="0.5" stopColor="#3b4f6b" /><stop offset="1" stopColor="#1f2c3e" /></linearGradient>
        <linearGradient id="srFloor" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stopColor="#18364d" /><stop offset="1" stopColor="#0a1622" /></linearGradient>
        <radialGradient id="srGlow" cx="0.5" cy="0.2" r="0.8"><stop offset="0" stopColor="#53e0ff" stopOpacity="0.28" /><stop offset="1" stopColor="#53e0ff" stopOpacity="0" /></radialGradient>
        <radialGradient id="alertG" cx="0.5" cy="0.5" r="0.5"><stop offset="0" stopColor="#ff3b3b" stopOpacity="0.85" /><stop offset="1" stopColor="#ff3b3b" stopOpacity="0" /></radialGradient>
      </defs>
      <rect width={1920} height={1080} fill="url(#srWall)" />
      {racks}
      {/* ceiling light strips and cable tray */}
      <rect x={0} y={0} width={1920} height={96} fill="#0a121d" />
      {[220, 700, 1180, 1660].map((x) => <rect key={x} x={x} y={36} width={340} height={14} rx={7} fill="#d8f6ff" opacity={0.9} />)}
      <rect x={0} y={88} width={1920} height={20} fill="#1c2a3d" /><rect x={0} y={104} width={1920} height={6} fill="#05080f" />
      {/* wall dashboard that insists everything is fine */}
      <g transform="translate(110,190)">
        <rect width={520} height={290} rx={14} fill="#06101a" stroke="#4c6786" strokeWidth={6} />
        <text x={30} y={56} fill="#6df2a6" fontFamily="Fredoka, sans-serif" fontWeight={600} fontSize={34}>ALL SYSTEMS OPERATIONAL</text>
        {[0, 1, 2, 3, 4, 5].map((i) => <g key={i}>
          <rect x={30} y={90 + i * 32} width={14} height={14} rx={7} fill="#3cff9d" />
          <rect x={58} y={92 + i * 32} width={120 + (i * 53) % 140} height={10} rx={5} fill="#2b4a63" />
          <rect x={360} y={92 + i * 32} width={130} height={10} rx={5} fill="#17314a" />
          <rect x={360} y={92 + i * 32} width={40 + ((i * 41 + Math.floor(frame / 9)) % 90)} height={10} rx={5} fill="#38b6ff" />
        </g>)}
      </g>
      {/* red alert lamp, very much not fine */}
      <g transform="translate(1450,170)">
        <circle r={150} fill="url(#alertG)" opacity={flash} />
        <rect x={-34} y={20} width={68} height={22} rx={6} fill="#26354a" stroke="#05080f" strokeWidth={4} />
        <path d="M-30,20 A30,30 0 0 1 30,20Z" fill="#ff4b4b" stroke="#05080f" strokeWidth={4} opacity={0.5 + flash * 0.5} />
        <g transform={`rotate(${spin} 0 8)`} opacity={0.4}><path d="M0,8 L190,-18 L190,34Z" fill="#ff6b6b" opacity={0.25} /></g>
      </g>
      {/* floor */}
      <rect x={0} y={690} width={1920} height={390} fill="url(#srFloor)" />
      <rect x={0} y={684} width={1920} height={10} fill="#53e0ff" opacity={0.35} />
      {Array.from({length: 14}).map((_, i) => <line key={i} x1={960 + (i - 6.5) * 60} y1={694} x2={960 + (i - 6.5) * 330} y2={1080} stroke="#53e0ff" strokeOpacity={0.12} strokeWidth={2} />)}
      {[740, 800, 880, 990].map((y) => <line key={y} x1={0} x2={1920} y1={y} y2={y} stroke="#53e0ff" strokeOpacity={0.1} strokeWidth={2} />)}
      <rect width={1920} height={1080} fill="url(#srGlow)" />
    </svg>
  );
};
