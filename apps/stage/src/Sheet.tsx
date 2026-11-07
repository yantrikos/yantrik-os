// Character sheet: a QA composition that lays a rig out across expressions,
// visemes or poses. Used by `npm run studio` and scripts/sheet.mjs.
import React from 'react';
import {AbsoluteFill, useCurrentFrame} from 'remotion';
import {RIGS, RigName, EXPRESSIONS, VISEMES, POSES} from './rigs';

export type SheetProps = {mode: 'cast' | 'expressions' | 'visemes' | 'poses'; rig: RigName};

export const Sheet: React.FC<SheetProps> = ({mode, rig}) => {
  const frame = useCurrentFrame();
  const names = Object.keys(RIGS) as RigName[];
  const cells: {rig: RigName; expression?: any; viseme?: any; pose?: any; label: string}[] =
    mode === 'cast' ? names.map((n) => ({rig: n, label: n, expression: 'happy'}))
    : mode === 'expressions' ? EXPRESSIONS.map((e) => ({rig, expression: e, label: e}))
    : mode === 'visemes' ? VISEMES.map((v) => ({rig, viseme: v, label: v, expression: 'neutral'}))
    : POSES.map((p) => ({rig, pose: p, label: p}));
  const cols = mode === 'cast' || mode === 'poses' ? 4 : 4;
  const w = 1920 / cols;
  return (
    <AbsoluteFill style={{background: 'linear-gradient(#3b3550,#1b1830)', flexWrap: 'wrap', flexDirection: 'row'}}>
      {cells.map((c, i) => {
        const R = RIGS[c.rig];
        return (
          <div key={i} style={{width: w, height: mode === 'cast' || mode === 'poses' ? 1080 : 540, position: 'relative', display: 'flex', justifyContent: 'center', alignItems: 'center'}}>
            <R frame={frame} fps={30} expression={c.expression} viseme={c.viseme} pose={c.pose} width={mode === 'cast' || mode === 'poses' ? 560 : 300} seed={i + 1} />
            <div style={{position: 'absolute', bottom: 8, color: '#fff', font: '28px sans-serif'}}>{c.label}</div>
          </div>
        );
      })}
    </AbsoluteFill>
  );
};
