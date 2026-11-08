// Server-room props: the alert lamp on the rack, the wall dashboard, the
// rack's glass door and the cable pile at the foot of the racks. Each is drawn
// in 1920x1080 scene space and takes its state from the episode.
import React from 'react';
import {beacon, Pulse, fitSize, FONT, clamp01, hash} from './common';
import type {PropArt} from './state';

// ------------------------------------------------------------ alert lamp
export const LAMP_X = 1432;

export const AlertLamp: PropArt = ({frame, state, text, since}) => {
  const lit = beacon(state, frame);
  const shown = text || 'ALERT';
  const fs = fitSize(shown, 112, 26, 11);
  const steady = state === 'steady';
  const core = 0.18 + 0.82 * lit;
  const on = lit > 0.5;
  return (
    <g transform={`translate(${LAMP_X},0)`}>
      <defs>
        <radialGradient id="lampDome" cx="0.38" cy="0.3" r="0.85">
          <stop offset="0" stopColor={on ? '#ffb4a6' : '#8a3a3a'} />
          <stop offset="0.4" stopColor={on ? '#ff3a30' : '#5e1618'} />
          <stop offset="1" stopColor={on ? '#9c0f14' : '#2a0809'} />
        </radialGradient>
        <radialGradient id="lampGlow" cx="0.5" cy="0.5" r="0.5">
          <stop offset="0" stopColor="#ff3b30" stopOpacity="0.9" />
          <stop offset="0.45" stopColor="#ff3b30" stopOpacity="0.28" />
          <stop offset="1" stopColor="#ff3b30" stopOpacity="0" />
        </radialGradient>
        <linearGradient id="lampHousing" x1="0" y1="0" x2="1" y2="0">
          <stop offset="0" stopColor="#2b394d" /><stop offset="0.5" stopColor="#3d5069" /><stop offset="1" stopColor="#222e3f" />
        </linearGradient>
      </defs>
      {/* glow on the wall and rack */}
      <circle cx={0} cy={176} r={steady ? 250 : 215} fill="url(#lampGlow)" opacity={lit * (steady ? 0.95 : 0.85)} />
      {/* housing and label plate */}
      <rect x={-72} y={146} width={144} height={128} rx={14} fill="url(#lampHousing)" stroke="#05080f" strokeWidth={5} />
      {[[-58, 160], [58, 160], [-58, 260], [58, 260]].map(([sx, sy], i) => (
        <g key={i}><circle cx={sx} cy={sy} r={5} fill="#17212f" stroke="#05080f" strokeWidth={1.5} /><line x1={sx - 3} y1={sy + 1} x2={sx + 3} y2={sy - 1} stroke="#4c607a" strokeWidth={1.5} /></g>
      ))}
      <rect x={-62} y={216} width={124} height={40} rx={7} fill="#eceff4" stroke="#05080f" strokeWidth={4} />
      <rect x={-57} y={221} width={114} height={30} rx={4} fill="none" stroke="#9aa6b8" strokeWidth={1.5} />
      <text x={0} y={236 + fs * 0.34} textAnchor="middle" fontFamily={FONT} fontWeight={700} fontSize={fs} letterSpacing={1.2} fill="#141a26">{shown.toUpperCase()}</text>
      {/* base ring, dome, highlight */}
      <rect x={-50} y={176} width={100} height={18} rx={6} fill="#4a5c75" stroke="#05080f" strokeWidth={4} />
      <path d="M-44,178 A44,44 0 0 1 44,178 Z" fill="url(#lampDome)" stroke="#05080f" strokeWidth={5} />
      <path d="M-30,166 A32,32 0 0 1 -6,144" fill="none" stroke="#fff" strokeWidth={5} strokeLinecap="round" opacity={0.25 + 0.5 * core} />
      <ellipse cx={0} cy={176} rx={40} ry={6} fill="#fff" opacity={0.18 * lit} />
      {/* rays */}
      {lit > 0.2 && [-70, -45, -20, 20, 45, 70].map((a) => {
        const r = (a * Math.PI) / 180;
        return <line key={a} x1={Math.sin(r) * 56} y1={176 - Math.cos(r) * 56} x2={Math.sin(r) * 78} y2={176 - Math.cos(r) * 78} stroke="#ff7a6e" strokeWidth={5} strokeLinecap="round" opacity={lit * 0.85} />;
      })}
      <g transform="translate(0,176)"><Pulse since={since} frame={frame} r0={46} r1={230} color="#ff6a5e" /></g>
    </g>
  );
};

// ------------------------------------------------------------- dashboard
const TONE: Record<string, string> = {ok: '#6df2a6', warn: '#ffc94d', alarm: '#ff5b5b'};

export const Dashboard: PropArt = ({frame, state, text, since}) => {
  const tone = TONE[state] ?? TONE.ok;
  const shown = (text || 'ALL SYSTEMS OPERATIONAL').toUpperCase();
  const fs = fitSize(shown, 440, 62, 20);
  const alarm = state === 'alarm';
  const borderLit = alarm ? 0.45 + 0.55 * beacon('blink', frame) : 1;
  const dots = (i: number) => (alarm && i < 3 ? '#ff5b5b' : state === 'warn' && i % 3 === 1 ? '#ffc94d' : '#3cff9d');
  const flicker = frame - since < 8 ? (frame % 2 ? 0.55 : 1) : 1;
  return (
    <g transform="translate(110,190)">
      <rect x={-8} y={-8} width={536} height={306} rx={20} fill="#0a121d" />
      <rect width={520} height={290} rx={14} fill="#06101a" stroke={alarm ? `rgba(255,91,91,${borderLit})` : '#4c6786'} strokeWidth={6} />
      <rect x={10} y={10} width={500} height={270} rx={8} fill={tone} opacity={alarm ? 0.06 + 0.06 * borderLit : 0.04} />
      <g opacity={flicker}>
        <text x={30} y={28 + fs * 0.78} fill={tone} fontFamily={FONT} fontWeight={600} fontSize={fs} letterSpacing={fs > 40 ? 3 : 0}>{shown}</text>
        {[0, 1, 2, 3, 4, 5].map((i) => (
          <g key={i}>
            <rect x={30} y={100 + i * 31} width={14} height={14} rx={7} fill={dots(i)} />
            <rect x={58} y={102 + i * 31} width={120 + (i * 53) % 140} height={10} rx={5} fill="#2b4a63" />
            <rect x={360} y={102 + i * 31} width={130} height={10} rx={5} fill="#17314a" />
            <rect x={360} y={102 + i * 31} width={40 + ((i * 41 + Math.floor(frame / 9)) % 90)} height={10} rx={5} fill={alarm ? '#ff8a7a' : '#38b6ff'} />
          </g>
        ))}
      </g>
      <rect x={12} y={12} width={496} height={34} rx={6} fill="#fff" opacity={0.04} />
    </g>
  );
};

// ------------------------------------------------------------ rack doors
const RX = 1330, RW = 205, RY = 110, RH = 560;

export const RackDoors: PropArt = ({frame, state, since}) => {
  const open = state === 'open';
  const k = clamp01((frame - since) / 16);
  const eased = 1 - Math.pow(1 - k, 3);
  const theta = ((open ? eased : 1 - eased) * 98 * Math.PI) / 180;   // 0 closed .. 98 degrees open
  const hx = RX + RW - 6, dw = RW - 14;
  const fx = hx - dw * Math.cos(theta), grow = 1 + 0.07 * Math.sin(theta);
  const top = RY + 10, bot = RY + RH - 10, mid = (top + bot) / 2, half = ((bot - top) / 2) * grow;
  const poly = `${hx},${top} ${fx},${mid - half} ${fx},${mid + half} ${hx},${bot}`;
  const sway = (i: number) => Math.sin(frame * 0.06 + i * 1.7) * 3;
  const cables = ['#ffb02e', '#38b6ff', '#3cff9d', '#ff5b7a', '#c9a4ff'];
  const spill = open ? eased : 0;
  return (
    <g>
      {/* cables spilling out once it is open */}
      {spill > 0.05 && cables.map((c, i) => {
        const x = RX + 44 + i * 30, drop = 60 + hash(i + 3) * 120 + (i % 2) * 40;
        return <path key={i} d={`M${x},${RY + 380 + i * 8} C${x + sway(i)},${RY + 440} ${x + 18 + sway(i + 1)},${RY + 440 + drop * 0.6} ${x + 8 + sway(i)},${RY + 400 + drop * spill}`}
          fill="none" stroke={c} strokeWidth={7} strokeLinecap="round" opacity={clamp01(spill * 1.4)} />;
      })}
      {/* the door: glass in a frame, hinged on the right */}
      <polygon points={poly} fill="rgba(140,190,240,0.13)" stroke="#a8bdd6" strokeWidth={5} strokeLinejoin="round" />
      {theta < 1.3 && (
        <g opacity={1 - theta / 1.3}>
          <line x1={fx + 24} y1={top + 14} x2={fx + 82} y2={mid - half * 0.3} stroke="#fff" strokeWidth={14} opacity={0.08} />
          <line x1={fx + 52} y1={top + 14} x2={fx + 92} y2={mid - half * 0.55} stroke="#fff" strokeWidth={8} opacity={0.07} />
          <rect x={fx + 7} y={mid - 26} width={9} height={52} rx={4} fill="#d8e3f2" stroke="#05080f" strokeWidth={2.5} />
          <rect x={fx + 22} y={top + 14} width={dw - 50} height={22} rx={3} fill="#0a1019" opacity={0.7} />
          <text x={fx + 34} y={top + 31} fontFamily={FONT} fontWeight={600} fontSize={15} fill="#8fb4d8" opacity={0.9}>RACK 07</text>
        </g>
      )}
    </g>
  );
};

// ------------------------------------------------------------ cable mess
const COLS = ['#ffb02e', '#38b6ff', '#3cff9d', '#ff5b7a', '#c9a4ff', '#e9edf3', '#ff8a3d'];

export const CableMess: PropArt = ({frame, state, since}) => {
  const pulled = state === 'pulled';
  const k = clamp01((frame - since) / 12);
  const zap = pulled && frame - since < 26 && frame % 6 < 3;
  const paths = COLS.map((_, i) => {
    const a = (i - 3) * 38, tw = hash(i + 9) * 70;
    return pulled
      ? `M${a - 40},-40 C${a - 80},${10 + (i % 3) * 14} ${a + 20},${46 - (i % 2) * 18} ${a + 110 + i * 6},${34 + (i % 3) * 5}`
      : `M${a - 20},-40 C${a - 110},${30 + tw} ${a + 90},${52 - tw * 0.4} ${a + 10},${4 + (i % 3) * 8} S${a - 60 + tw},-8 ${a + 70},${30 + (i % 3) * 8}`;
  });
  return (
    <g transform="translate(900,690)">
      <ellipse cx={0} cy={44} rx={190} ry={22} fill="#000" opacity={0.3} />
      {paths.map((d, i) => <path key={`o${i}`} d={d} fill="none" stroke="#05080f" strokeWidth={15} strokeLinecap="round" />)}
      {paths.map((d, i) => <path key={`c${i}`} d={d} fill="none" stroke={COLS[i]} strokeWidth={9} strokeLinecap="round" />)}
      {/* cable tie, power strip, and the plug that has come out */}
      <rect x={-34} y={-6} width={22} height={9} rx={3} fill="#e9edf3" stroke="#05080f" strokeWidth={2} transform="rotate(-14)" />
      <rect x={-150} y={26} width={104} height={24} rx={6} fill="#d9dde6" stroke="#05080f" strokeWidth={4} />
      {[0, 1, 2, 3].map((i) => <rect key={i} x={-138 + i * 24} y={32} width={14} height={12} rx={2} fill={pulled && i === 1 ? '#05080f' : '#3b4558'} />)}
      {pulled ? (
        <g transform={`translate(${90 + 40 * k},34)`}>
          <rect x={-20} y={-12} width={40} height={24} rx={6} fill="#e9edf3" stroke="#05080f" strokeWidth={4} />
          <rect x={20} y={-8} width={12} height={5} fill="#c9ced8" stroke="#05080f" strokeWidth={1.5} />
          <rect x={20} y={3} width={12} height={5} fill="#c9ced8" stroke="#05080f" strokeWidth={1.5} />
          {zap && <path d="M34,-18 l8,10 l-7,2 l10,12 M44,4 l9,4" fill="none" stroke="#ffe27a" strokeWidth={4} strokeLinecap="round" />}
        </g>
      ) : (
        <rect x={-20} y={-6} width={40} height={22} rx={6} fill="#e9edf3" stroke="#05080f" strokeWidth={4} transform="translate(-56,28)" />
      )}
    </g>
  );
};
