// Lower-third captions: speaker chip plus the words, lit up as they are spoken.
import React from 'react';
import {spring, useVideoConfig} from './anim';

export const Captions: React.FC<{line: any; g: number; color: string}> = ({line, g, color}) => {
  const {fps} = useVideoConfig();
  const rel = g - line.start;
  if (rel < -3) return null;
  const enter = spring({frame: rel + 4, fps, config: {damping: 14, stiffness: 180}, durationInFrames: 14});
  const words: string[] = line.text.split(/\s+/);
  const total = words.reduce((a, w) => a + w.length + 2, 0);
  const span = Math.max(1, line.speakEnd - line.speakStart);
  let acc = 0;
  const marks = words.map((w) => { const s = acc / total; acc += w.length + 2; return line.speakStart + s * span; });
  return (
    <div style={{position: 'absolute', left: 0, right: 0, bottom: 26, display: 'flex', justifyContent: 'center',
      transform: `translateY(${(1 - enter) * 40}px) scale(${0.96 + 0.04 * enter})`, opacity: enter}}>
      <div style={{maxWidth: 1480, padding: '10px 30px 14px', borderRadius: 26, background: 'rgba(14,12,28,0.78)', border: `4px solid ${color}`,
        boxShadow: '0 10px 40px rgba(0,0,0,.45)', fontFamily: 'Fredoka, sans-serif', textAlign: 'center'}}>
        <div style={{display: 'inline-block', marginBottom: 2, padding: '0 18px', borderRadius: 14, background: color, color: '#16122a',
          fontWeight: 700, fontSize: 26, letterSpacing: 2}}>{line.speaker}</div>
        <div style={{fontSize: 46, fontWeight: 600, lineHeight: 1.15, color: '#fff'}}>
          {words.map((w, i) => (
            <span key={i} style={{color: rel >= marks[i] ? '#ffffff' : 'rgba(255,255,255,0.5)', textShadow: rel >= marks[i] && rel < marks[i] + 5 ? `0 0 18px ${color}` : 'none'}}>{w}{' '}</span>
          ))}
        </div>
      </div>
    </div>
  );
};
