// One scene: set, staged characters, camera, captions and entry/exit transitions.
import React from 'react';
import {useCurrentFrame} from './anim';
import {RIGS, RigName, Expression, Pose, Viseme} from './rigs';
import {SETS} from './sets';
import {Captions} from './Captions';
import {FOOT_Y, rigWidth, shot, lerpCam, ease, fit, Cam} from './layout';
import {castColor} from './castInfo';
import {placeAt, Move} from './blocking';
import {planShots, Seg} from './director';
import {propsAt, insertOf} from './props/state';

type Line = {speaker: string; emotion: Expression; pose: Pose | null; camera: string | null; react: Record<string, Expression>; begin: number;
  start: number; audioFrames: number; end: number; audio: string; visemes: string; energy: number[]; text: string; speakStart: number; speakEnd: number};
type Scene = {set: string; camera: string; transition: string; exit: string | null; present: string[]; start: number; end: number; lines: Line[];
  startX: Record<string, number>; moves: Move[]; propsStart: any; propEvents: any[]};

const VMAP: Record<string, Viseme> = {'.': 'rest', A: 'A', E: 'E', O: 'O', U: 'U', M: 'M', F: 'F'};

export const SceneView: React.FC<{scene: Scene; cast: any[]; accent: string}> = ({scene, cast, accent}) => {
  const f = useCurrentFrame(); // local to the scene Sequence
  const g = scene.start + f; // global
  const n = scene.present.length;
  const lines = scene.lines;
  let li = 0;
  for (let i = 0; i < lines.length; i++) if (g >= lines[i].begin) li = i;
  const line = lines[li];
  const inAudio = g >= line.start && g < line.start + line.audioFrames;
  const speakerIdx = Math.max(0, scene.present.indexOf(line.speaker));

  // who stands where right now (walkers are mid-stride)
  const placed = scene.present.map((nm) => placeAt(nm, scene.startX[nm], scene.moves, g));
  const xs = placed.map((p) => p.x);

  // camera: the director's plan, easing between shots and cutting to and from inserts
  const segs = React.useMemo(() => planShots(scene), [scene]);
  let si = 0;
  for (let i = 0; i < segs.length; i++) if (g >= segs[i].from) si = i;
  const curSeg = segs[si], prevSeg = segs[si - 1];
  const camOf = (sg: Seg): Cam => {
    const p = Math.min(1, Math.max(0, (g - sg.runFrom) / Math.max(1, sg.runTo - sg.runFrom)));
    if (sg.kind === 'insert') {
      const ins = insertOf(scene.set, sg.prop ?? '');
      return ins ? fit({cx: ins.cx, cy: ins.cy, z: ins.z * (1 + 0.06 * p)}) : shot('wide', 0, 0, xs, 0);
    }
    return shot(sg.kind, sg.subject, sg.partner, xs, p);
  };
  const cut = !prevSeg || prevSeg.kind === 'insert' || curSeg.kind === 'insert';
  const cam = cut ? camOf(curSeg) : lerpCam(camOf(prevSeg), camOf(curSeg), ease((g - (curSeg.from - 4)) / 12));
  const insertShot = curSeg.kind === 'insert';
  const propViews = propsAt(scene, g);

  const SetArt = SETS[scene.set] ?? SETS['server-room'];
  const size = rigWidth(n);

  // entry fade/wipe and exit fade
  const enter = scene.transition;
  const fadeIn = enter === 'fade' ? 1 - Math.min(1, f / 10) : 0;
  const fadeOut = scene.exit === 'fade' ? Math.max(0, 1 - (scene.end - g) / 8) : 0;
  const wipeP = enter === 'wipe' ? f / 16 : 2;

  return (
    <div style={{position: 'absolute', inset: 0, overflow: 'hidden', background: '#000'}}>
      <div style={{position: 'absolute', left: 0, top: 0, width: 1920, height: 1080, transformOrigin: '0 0',
        transform: `translate(${960 - cam.cx * cam.z}px, ${540 - cam.cy * cam.z}px) scale(${cam.z})`, opacity: enter === 'wipe' && wipeP < 0.5 ? 0 : 1}}>
        <SetArt frame={g} props={propViews} />
        {!insertShot && scene.present.map((name, i) => {
          const member = cast.find((c) => c.name === name)!;
          const R = RIGS[member.rig as RigName];
          const x = xs[i], pl = placed[i];
          const speaking = name === line.speaker;
          let expression: Expression = speaking ? (g >= line.start - 4 ? line.emotion : 'neutral') : line.react?.[name] ?? 'neutral';
          let pose: Pose = 'idle';
          let viseme: Viseme = 'rest';
          let energy = 0;
          if (speaking) {
            pose = line.pose ?? (inAudio || (g >= line.start && g < line.start + line.audioFrames + 6) ? 'talk-gesture' : 'idle');
            if (inAudio) {
              viseme = VMAP[line.visemes[g - line.start]] ?? 'rest';
              energy = line.energy[g - line.start] ?? 0;
            }
          }
          if (pl.walking) pose = 'idle';
          const flip = pl.walking ? pl.dir < 0 : n > 1 && x > 960;
          const scale = speaking ? 1.06 : 0.97;
          const look = Math.max(-1, Math.min(1, (xs[speakerIdx] - x) / 700));
          return (
            <div key={name} style={{position: 'absolute', left: x - size / 2, top: FOOT_Y - (565 / 600) * size * 1.5, width: size, height: size * 1.5,
              transform: `scale(${scale})`, transformOrigin: `50% ${(565 / 600) * 100}%`, filter: speaking ? 'none' : 'brightness(0.84) saturate(0.92)'}}>
              <R frame={g} fps={30} expression={expression} viseme={viseme} pose={pose} energy={energy} palette={member.palette} accessory={member.accessory}
                flip={flip} look={speaking ? 0 : flip ? -look : look} seed={i * 3 + 1} width={size} walk={pl.walking ? pl.phase : undefined} />
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
      <div style={{position: 'absolute', inset: 0,
        background: insertShot ? 'radial-gradient(ellipse at center, transparent 40%, rgba(0,0,0,0.6) 100%)' : 'radial-gradient(ellipse at center, transparent 55%, rgba(0,0,0,0.45) 100%)', pointerEvents: 'none'}} />
    </div>
  );
};
