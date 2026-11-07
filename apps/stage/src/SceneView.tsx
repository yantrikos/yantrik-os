// One scene: set, staged characters, camera, captions and entry/exit transitions.
import React from 'react';
import {Audio, Sequence, staticFile, useCurrentFrame} from 'remotion';
import {RIGS, RigName, Expression, Pose, Viseme} from './rigs';
import {SETS} from './sets';
import {Captions} from './Captions';
import {FOOT_Y, rigWidth, slotX, shot, lerpCam, ease, Camera} from './layout';
import {castColor} from './castInfo';

type Line = {speaker: string; emotion: Expression; pose: Pose | null; camera: Camera | null; react: Record<string, Expression>;
  start: number; audioFrames: number; end: number; audio: string; visemes: string; energy: number[]; text: string; speakStart: number; speakEnd: number};
type Scene = {set: string; camera: Camera; transition: string; present: string[]; start: number; end: number; lines: Line[]};

const VMAP: Record<string, Viseme> = {'.': 'rest', A: 'A', E: 'E', O: 'O', U: 'U', M: 'M', F: 'F'};

export const SceneView: React.FC<{scene: Scene; cast: any[]; nextTransition: string | null; accent: string}> = ({scene, cast, nextTransition, accent}) => {
  const f = useCurrentFrame(); // local to the scene Sequence
  const g = scene.start + f; // global
  const n = scene.present.length;
  const lines = scene.lines;
  let li = 0;
  for (let i = 0; i < lines.length; i++) if (g >= lines[i].start - 4) li = i;
  const line = lines[li];
  const inAudio = g >= line.start && g < line.start + line.audioFrames;
  const speakerIdx = Math.max(0, scene.present.indexOf(line.speaker));
  const partnerOf = (l: Line, idx: number) => {
    const prev = lines[lines.indexOf(l) - 1];
    const cand = prev && prev.speaker !== l.speaker && scene.present.includes(prev.speaker) ? scene.present.indexOf(prev.speaker) : -1;
    if (cand >= 0) return cand;
    return idx === 0 ? Math.min(1, n - 1) : idx - 1;
  };
  const camFor = (l: Line, i: number) => {
    const idx = Math.max(0, scene.present.indexOf(l.speaker));
    return shot(l.camera ?? scene.camera, idx, partnerOf(l, idx), n, (g - scene.start) / (scene.end - scene.start));
  };
  const cur = camFor(line, li);
  const prevLine = lines[li - 1];
  const t = ease((g - (line.start - 8)) / 12);
  const cam = prevLine ? lerpCam(camFor(prevLine, li - 1), cur, t) : cur;

  const SetArt = SETS[scene.set] ?? SETS['server-room'];
  const size = rigWidth(n);
  const speakFrame = g - line.start;

  // entry fade/wipe and exit fade
  const enter = scene.transition;
  const fadeIn = enter === 'fade' ? 1 - Math.min(1, f / 10) : 0;
  const fadeOut = nextTransition === 'fade' ? Math.max(0, 1 - (scene.end - g) / 8) : 0;
  const wipeP = enter === 'wipe' ? f / 16 : 2;

  return (
    <div style={{position: 'absolute', inset: 0, overflow: 'hidden', background: '#000'}}>
      <div style={{position: 'absolute', left: 0, top: 0, width: 1920, height: 1080, transformOrigin: '0 0',
        transform: `translate(${960 - cam.cx * cam.z}px, ${540 - cam.cy * cam.z}px) scale(${cam.z})`, opacity: enter === 'wipe' && wipeP < 0.5 ? 0 : 1}}>
        <SetArt frame={g} />
        {scene.present.map((name, i) => {
          const member = cast.find((c) => c.name === name)!;
          const R = RIGS[member.rig as RigName];
          const x = slotX(i, n);
          const speaking = name === line.speaker;
          let expression: Expression = speaking ? line.emotion : line.react?.[name] ?? 'neutral';
          let pose: Pose = 'idle';
          let viseme: Viseme = 'rest';
          let energy = 0;
          if (speaking) {
            pose = line.pose ?? (inAudio || g < line.start + line.audioFrames + 6 ? 'talk-gesture' : 'idle');
            if (inAudio) {
              const k = speakFrame;
              viseme = VMAP[line.visemes[k]] ?? 'rest';
              energy = line.energy[k] ?? 0;
            }
          }
          const flip = n > 1 && x > 960;
          const scale = speaking ? 1.06 : 0.97;
          const look = Math.max(-1, Math.min(1, (slotX(speakerIdx, n) - x) / 700));
          return (
            <div key={name} style={{position: 'absolute', left: x - size / 2, top: FOOT_Y - (565 / 600) * size * 1.5, width: size, height: size * 1.5,
              transform: `scale(${scale})`, transformOrigin: `50% ${(565 / 600) * 100}%`, filter: speaking ? 'none' : 'brightness(0.84) saturate(0.92)'}}>
              {speaking && <div style={{position: 'absolute', left: '8%', width: '84%', top: '88%', height: '10%', borderRadius: '50%', background: `radial-gradient(${accent}88, transparent 70%)`, opacity: 0.0}} />}
              <R frame={g} fps={30} expression={expression} viseme={viseme} pose={pose} energy={energy} palette={member.palette} accessory={member.accessory}
                flip={flip} look={speaking ? 0 : flip ? -look : look} seed={i * 3 + 1} width={size} />
            </div>
          );
        })}
      </div>
      <Captions line={line} g={g} color={castColor(cast, line.speaker)} />
      {wipeP < 1.2 && wipeP >= 0 && (
        <div style={{position: 'absolute', top: -100, bottom: -100, width: 1500, left: -1500 + wipeP * 3500, transform: 'skewX(-14deg)',
          background: `linear-gradient(90deg, ${accent}, #ffffff 90%)`, boxShadow: '0 0 60px rgba(0,0,0,.4)'}} />
      )}
      {(fadeIn > 0 || fadeOut > 0) && <div style={{position: 'absolute', inset: 0, background: '#000', opacity: Math.max(fadeIn, fadeOut)}} />}
      <div style={{position: 'absolute', inset: 0, background: 'radial-gradient(ellipse at center, transparent 55%, rgba(0,0,0,0.45) 100%)', pointerEvents: 'none'}} />
      {scene.lines.map((l) => (
        <Sequence key={l.start} from={l.start - scene.start} durationInFrames={l.audioFrames + 2}>
          <Audio src={staticFile(l.audio)} />
        </Sequence>
      ))}
    </div>
  );
};
