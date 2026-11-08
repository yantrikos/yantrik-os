// The episode: title card, scenes, credits and audio, driven by two JSON
// inputs (episode.normalized.json and timeline.json) built by the voice step.
import React, {useEffect, useState} from 'react';
import {AbsoluteFill, Audio, Sequence, continueRender, delayRender, staticFile} from 'remotion';
import {SceneView} from './SceneView';
import {TitleCard, Credits} from './Cards';
import {castColor} from './castInfo';

export type EpisodeProps = {episode: any; timeline: any};

const useFonts = () => {
  const [h] = useState(() => delayRender('fonts'));
  useEffect(() => {
    const ff = new FontFace('Fredoka', `url(${staticFile('fonts/Fredoka.ttf')})`, {weight: '300 700'});
    ff.load().then((f) => { document.fonts.add(f); continueRender(h); }).catch(() => continueRender(h));
  }, [h]);
};

export const Episode: React.FC<EpisodeProps> = ({episode, timeline}) => {
  useFonts();
  if (!episode || !timeline) return <AbsoluteFill style={{background: '#150f33', color: '#fff', fontSize: 60, justifyContent: 'center', alignItems: 'center'}}>No episode loaded</AbsoluteFill>;
  const t = timeline;
  const sceneAccent = (s: any) => castColor(episode.cast, s.lines[0].speaker);
  // music ducking: lower the bed while someone talks
  const talking = new Uint8Array(t.totalFrames + 2);
  for (const s of t.scenes) for (const l of s.lines) for (let k = l.start; k < l.start + l.audioFrames; k++) talking[k] = 1;
  const base = (t.music?.volume ?? 0.5) * 0.5;
  const duck = (f: number) => {
    let near = 0;
    for (let k = -6; k <= 6; k++) near = Math.max(near, talking[Math.max(0, f + k)] ?? 0);
    return near ? 0.42 : 1;
  };
  return (
    <AbsoluteFill style={{background: '#000'}}>
      <Sequence from={t.titleStart} durationInFrames={t.titleFrames}><TitleCard show={episode.show} title={episode.title} cast={episode.cast} frames={t.titleFrames} /></Sequence>
      {t.scenes.map((s: any, i: number) => (
        <Sequence key={i} from={s.start} durationInFrames={s.end - s.start}>
          <SceneView scene={s} cast={episode.cast} accent={sceneAccent(s)} />
        </Sequence>
      ))}
      <Sequence from={t.creditsStart} durationInFrames={t.creditsFrames}><Credits cast={episode.cast} title={episode.title} frames={t.creditsFrames} /></Sequence>
      {(t.sfx ?? []).map((c: any, i: number) => (
        <Sequence key={`sfx${i}`} from={c.at}><Audio src={staticFile(c.file)} volume={c.volume} /></Sequence>
      ))}
      {t.music && <Audio src={staticFile(t.music.file)} volume={(f) => base * duck(f)} />}
    </AbsoluteFill>
  );
};
