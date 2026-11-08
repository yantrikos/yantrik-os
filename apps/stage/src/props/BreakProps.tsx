// Break-room props: the coffee machine, the vending machine and the wall clock.
import React from 'react';
import {fitSize, FONT, clamp01, beacon} from './common';
import type {PropArt} from './state';

// --------------------------------------------------------- coffee machine
export const CoffeeMachine: PropArt = ({frame, state, text, since}) => {
  const brewing = state === 'brewing', empty = state === 'empty';
  const label = (text || (brewing ? 'BREWING' : empty ? 'EMPTY' : 'READY')).toUpperCase();
  const tone = empty ? '#ffb02e' : brewing ? '#6db8ff' : '#6df2a6';
  const blink = empty ? beacon('blink', frame) : 1;
  const t = frame - since;
  const level = brewing ? clamp01(t / 110) * 0.85 : 0;
  const fs = fitSize(label, 104, 28, 12);
  const steam = (i: number) => {
    const s = ((frame + i * 17) % 60) / 60;
    return <circle key={i} cx={1470 + Math.sin(s * 9 + i) * 9} cy={266 - s * 100} r={8 + s * 11} fill="#fff" opacity={0.4 * (1 - s) * (empty ? 0 : brewing ? 1 : 0.6)} />;
  };
  return (
    <g>
      <defs>
        <linearGradient id="cmBody" x1="0" y1="0" x2="1" y2="0"><stop offset="0" stopColor="#3a4157" /><stop offset="0.5" stopColor="#2a2f40" /><stop offset="1" stopColor="#1d2130" /></linearGradient>
      </defs>
      <ellipse cx={1470} cy={488} rx={110} ry={9} fill="#000" opacity={0.3} />
      <rect x={1385} y={292} width={170} height={194} rx={16} fill="url(#cmBody)" stroke="#0b0d14" strokeWidth={6} />
      <rect x={1385} y={292} width={170} height={36} rx={16} fill="#4a5270" stroke="#0b0d14" strokeWidth={6} />
      <rect x={1405} y={338} width={130} height={52} rx={8} fill="#0b0d14" stroke="#05060a" strokeWidth={3} />
      <text x={1470} y={338 + 26 + fs * 0.35} textAnchor="middle" fontFamily={FONT} fontWeight={600} fontSize={fs} fill={tone} opacity={blink}>{label}</text>
      {[0, 1, 2].map((i) => <circle key={i} cx={1420 + i * 22} cy={406} r={7} fill={i === (brewing ? 1 : 0) ? tone : '#556'} stroke="#05060a" strokeWidth={2} />)}
      <rect x={1490} y={398} width={46} height={16} rx={5} fill="#556" stroke="#05060a" strokeWidth={2} />
      {/* spout, stream, cup and drip tray */}
      <rect x={1452} y={418} width={36} height={14} rx={4} fill="#15181f" stroke="#05060a" strokeWidth={2.5} />
      {brewing && <rect x={1467} y={430} width={6} height={52 - (1 - level) * 18} rx={3} fill="#6a3d1f" opacity={0.95} />}
      <g transform="translate(1470,452)">
        <path d="M-26,0 L-22,34 L22,34 L26,0Z" fill="#f4efe6" stroke="#05080f" strokeWidth={4} strokeLinejoin="round" />
        {level > 0.01 && <path d={`M${-26 + 4 * (1 - level)},${34 - level * 34} L${-22},34 L22,34 L${26 - 4 * (1 - level)},${34 - level * 34}Z`} fill="#6a3d1f" />}
        <path d="M26,8 Q42,12 36,26 Q33,31 23,30" fill="none" stroke="#05080f" strokeWidth={4} strokeLinecap="round" />
        {empty && <text x={0} y={22} textAnchor="middle" fontFamily={FONT} fontWeight={700} fontSize={22} fill="#7a8196">?</text>}
      </g>
      <rect x={1410} y={484} width={120} height={7} rx={3} fill="#15181f" />
      {[0, 1, 2].map(steam)}
    </g>
  );
};

// -------------------------------------------------------- vending machine
const SNACKS = ['#ff6b6b', '#ffd35c', '#5cc8ff', '#a78bfa', '#6df2a6'];

export const VendingMachine: PropArt = ({frame, state, text}) => {
  const off = state === 'off', stuck = state === 'stuck';
  const label = (text || 'A4').toUpperCase();
  const flash = stuck ? beacon('blink', frame) : 1;
  return (
    <g transform="translate(1590,240)">
      <defs>
        <linearGradient id="vmBody" x1="0" y1="0" x2="1" y2="0"><stop offset="0" stopColor="#d6334b" /><stop offset="1" stopColor="#8f1d33" /></linearGradient>
      </defs>
      <rect width={270} height={470} rx={14} fill="url(#vmBody)" stroke="#3b0f1b" strokeWidth={7} />
      <rect x={20} y={24} width={170} height={330} rx={8} fill="#171c2a" stroke="#3b0f1b" strokeWidth={5} />
      {[0, 1, 2, 3].map((r) => (
        <g key={r}>
          <line x1={26} x2={184} y1={92 + r * 80} y2={92 + r * 80} stroke="#aebbd0" strokeWidth={4} />
          {[0, 1, 2, 3].map((c) => ((r + c) % 3 === 2 || (stuck && r === 0 && c === 1) ? null
            : <rect key={c} x={34 + c * 38} y={48 + r * 80} width={28} height={40} rx={5} fill={SNACKS[(r * 4 + c) % 5]} />))}
        </g>
      ))}
      {stuck && <g transform="translate(86,56) rotate(-18 14 6)"><rect width={28} height={40} rx={5} fill="#ffd35c" stroke="#3b0f1b" strokeWidth={2} /><rect x={4} y={14} width={20} height={10} fill="#fff" opacity={0.6} /></g>}
      <rect x={208} y={34} width={44} height={128} rx={6} fill="#2a2f40" />
      <rect x={213} y={40} width={34} height={26} rx={3} fill="#0b0d14" />
      {!off && <text x={230} y={59} textAnchor="middle" fontFamily={FONT} fontWeight={700} fontSize={fitSize(label, 30, 18, 9)} fill={stuck ? '#ff6b6b' : '#6df2a6'} opacity={flash}>{label}</text>}
      {[0, 1, 2].map((i) => <rect key={i} x={216} y={76 + i * 28} width={28} height={20} rx={3} fill={i === 0 && !off ? '#6df2a6' : '#556'} />)}
      <rect x={34} y={386} width={200} height={54} rx={8} fill="#0b0d14" />
      {stuck && <rect x={34} y={386} width={200} height={54} rx={8} fill="none" stroke="#ff6b6b" strokeWidth={2} opacity={flash * 0.7} />}
      {off && <rect width={270} height={470} rx={14} fill="#05060d" opacity={0.62} />}
      {!off && <rect x={20} y={24} width={170} height={330} rx={8} fill="#fff" opacity={0.04} />}
    </g>
  );
};

// ------------------------------------------------------------- wall clock
const parseTime = (t: string) => {
  const m = /^\s*(\d{1,2})\s*[:.]\s*(\d{2})\s*$/.exec(t);
  return m ? {h: Number(m[1]) % 12, min: Math.min(59, Number(m[2]))} : {h: 4, min: 59};
};

export const WallClock: PropArt = ({frame, state, text, since}) => {
  const {h, min} = parseTime(text || '4:59');
  const t = Math.max(0, frame - since);
  const racing = state === 'racing', ticking = state === 'ticking';
  const minute = racing ? min * 6 + t * 9 : min * 6 + (ticking ? (t / 30 / 60) * 6 : 0);
  const hour = racing ? (h + min / 60) * 30 + t * 0.75 : (h + min / 60) * 30;
  const second = racing ? t * 24 : ticking ? Math.floor(t / 30) * 6 : null;
  return (
    <g transform="translate(1330,230)">
      <circle r={66} fill="#fff8e8" stroke="#3a2a22" strokeWidth={8} />
      <circle r={56} fill="none" stroke="#e8dcc0" strokeWidth={2} />
      {[...Array(12)].map((_, i) => <line key={i} x1={0} y1={i % 3 === 0 ? -54 : -50} x2={0} y2={-44} stroke="#3a2a22" strokeWidth={i % 3 === 0 ? 5 : 3} transform={`rotate(${i * 30})`} />)}
      <line x1={0} y1={6} x2={0} y2={-32} stroke="#3a2a22" strokeWidth={7} strokeLinecap="round" transform={`rotate(${hour})`} />
      <line x1={0} y1={8} x2={0} y2={-48} stroke="#3a2a22" strokeWidth={4} strokeLinecap="round" transform={`rotate(${minute})`} />
      {second !== null && <line x1={0} y1={12} x2={0} y2={-50} stroke="#d6334b" strokeWidth={2} strokeLinecap="round" transform={`rotate(${second})`} />}
      <circle r={6} fill="#d6334b" />
    </g>
  );
};
