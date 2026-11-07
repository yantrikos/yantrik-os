// Title card and end credits.
import React from 'react';
import {interpolate, spring, useCurrentFrame, useVideoConfig} from 'remotion';
import {RIGS, RigName} from './rigs';
import {ROLE, castColor} from './castInfo';

const FONT = 'Fredoka, sans-serif';

const Backdrop: React.FC<{frame: number; a: string; b: string}> = ({frame, a, b}) => (
  <div style={{position: 'absolute', inset: 0, background: `radial-gradient(ellipse at 50% 40%, ${a}, ${b})`, overflow: 'hidden'}}>
    <svg width={1920} height={1080} style={{position: 'absolute', inset: 0}}>
      {Array.from({length: 14}).map((_, i) => (
        <path key={i} d={`M960,420 L${960 + Math.cos(i * 0.449 + frame * 0.004) * 2400},${420 + Math.sin(i * 0.449 + frame * 0.004) * 2400} L${960 + Math.cos(i * 0.449 + 0.2 + frame * 0.004) * 2400},${420 + Math.sin(i * 0.449 + 0.2 + frame * 0.004) * 2400}Z`} fill="#fff" opacity={i % 2 ? 0.035 : 0} />
      ))}
      {Array.from({length: 26}).map((_, i) => {
        const x = (i * 211) % 1920, y = 1080 - ((frame * (0.6 + (i % 5) * 0.3) + i * 97) % 1200);
        return <circle key={i} cx={x} cy={y} r={3 + (i % 4) * 2} fill="#fff" opacity={0.14} />;
      })}
    </svg>
  </div>
);

export const TitleCard: React.FC<{show: string; title: string; cast: any[]; frames: number}> = ({show, title, cast, frames}) => {
  const f = useCurrentFrame();
  const {fps} = useVideoConfig();
  const letters = show.split('');
  const out = interpolate(f, [frames - 10, frames], [0, 1], {extrapolateLeft: 'clamp'});
  const sub = spring({frame: f - 30, fps, config: {damping: 12}});
  const n = cast.length;
  return (
    <div style={{position: 'absolute', inset: 0}}>
      <Backdrop frame={f} a="#5a3d9a" b="#150f33" />
      <div style={{position: 'absolute', top: 120, left: 0, right: 0, textAlign: 'center', fontFamily: FONT, fontWeight: 700, fontSize: 190, letterSpacing: 6, whiteSpace: 'pre'}}>
        {letters.map((ch, i) => {
          const s = spring({frame: f - i * 2, fps, config: {damping: 9, stiffness: 160}});
          return <span key={i} style={{display: 'inline-block', transform: `translateY(${(1 - s) * -260}px) rotate(${(1 - s) * (i % 2 ? 25 : -25)}deg) scale(${0.5 + s * 0.5})`, opacity: Math.min(1, s * 2),
            color: '#fff', WebkitTextStroke: '10px #1b1238', paintOrder: 'stroke fill', textShadow: '0 12px 0 #ff9a4a, 0 22px 40px rgba(0,0,0,.5)'}}>{ch}</span>;
        })}
      </div>
      <div style={{position: 'absolute', top: 400, left: 0, right: 0, display: 'flex', justifyContent: 'center', transform: `scale(${sub}) rotate(${(1 - sub) * -4}deg)`, opacity: sub}}>
        <div style={{padding: '8px 54px 14px', background: '#ffd35c', borderRadius: 18, boxShadow: '0 8px 0 #c99412, 0 20px 40px rgba(0,0,0,.4)', fontFamily: FONT, fontWeight: 600, fontSize: 64, color: '#2a1d00'}}>{title}</div>
      </div>
      <div style={{position: 'absolute', left: 0, right: 0, bottom: 40, height: 520}}>
        {cast.map((m, i) => {
          const R = RIGS[m.rig as RigName];
          const s = spring({frame: f - 22 - i * 5, fps, config: {damping: 11, stiffness: 140}});
          const w = 360;
          const x = n === 1 ? 960 : 330 + (i * (1920 - 660)) / (n - 1);
          return <div key={m.name} style={{position: 'absolute', left: x - w / 2, bottom: -140 + (1 - s) * -500, width: w}}>
            <R frame={f} fps={fps} expression="happy" pose={i % 2 ? 'idle' : 'talk-gesture'} energy={0.3} palette={m.palette} accessory={m.accessory} flip={i >= n / 2} seed={i * 3 + 1} width={w} />
          </div>;
        })}
      </div>
      <div style={{position: 'absolute', inset: 0, background: '#000', opacity: out}} />
    </div>
  );
};

export const Credits: React.FC<{cast: any[]; title: string; frames: number}> = ({cast, title, frames}) => {
  const f = useCurrentFrame();
  const {fps} = useVideoConfig();
  const fadeIn = interpolate(f, [0, 12], [1, 0], {extrapolateRight: 'clamp'});
  const fadeOut = interpolate(f, [frames - 14, frames], [0, 1], {extrapolateLeft: 'clamp'});
  return (
    <div style={{position: 'absolute', inset: 0, fontFamily: FONT}}>
      <Backdrop frame={f + 400} a="#2b2158" b="#0d0a1f" />
      <div style={{position: 'absolute', left: 150, top: 120, width: 1000, color: '#fff'}}>
        <div style={{fontSize: 44, letterSpacing: 10, color: '#ffd35c', fontWeight: 600, opacity: spring({frame: f, fps})}}>STARRING</div>
        {cast.map((m, i) => {
          const s = spring({frame: f - 8 - i * 6, fps, config: {damping: 14}});
          return <div key={m.name} style={{marginTop: 28, transform: `translateX(${(1 - s) * -120}px)`, opacity: s}}>
            <span style={{fontSize: 84, fontWeight: 700, color: castColor(cast, m.name), WebkitTextStroke: '2px #0d0a1f'}}>{m.name}</span>
            <span style={{fontSize: 36, marginLeft: 28, color: '#cfc8ff'}}>{ROLE[m.rig] ?? ''}</span>
          </div>;
        })}
        <div style={{marginTop: 60, fontSize: 34, color: '#cfc8ff', opacity: spring({frame: f - 40, fps}), lineHeight: 1.5}}>
          Written by a small model.<br />Drawn, voiced and filmed by Yantrik OS.
        </div>
      </div>
      <div style={{position: 'absolute', right: 60, bottom: -90, display: 'flex', gap: 0}}>
        {cast.map((m, i) => {
          const R = RIGS[m.rig as RigName];
          return <div key={m.name} style={{width: 270, marginLeft: i ? -10 : 0}}><R frame={f} fps={fps} expression={i % 2 ? 'laughing' : 'happy'} pose="idle" palette={m.palette} accessory={m.accessory} flip={i >= cast.length / 2} seed={i + 4} width={270} /></div>;
        })}
      </div>
      <div style={{position: 'absolute', right: 70, top: 70, fontSize: 30, color: '#9d95d6', letterSpacing: 4}}>{title.toUpperCase()}</div>
      <div style={{position: 'absolute', inset: 0, background: '#000', opacity: Math.max(fadeIn, fadeOut)}} />
    </div>
  );
};
