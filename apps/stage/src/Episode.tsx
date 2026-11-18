// The episode: title card, scenes and credits, driven by two JSON inputs
// (episode.normalized.json and timeline.json) built by the voice step.
// Sound is mixed by scripts/audio.mjs from the same timeline; the Fredoka
// font is loaded by the page (scripts/page.mjs) before the first frame.
import React from 'react';
import {AbsoluteFill, Sequence} from './anim';
import {SceneView} from './SceneView';
import {TitleCard, Credits} from './Cards';
import {castColor} from './castInfo';

export type EpisodeProps = {episode: any; timeline: any};

export const Episode: React.FC<EpisodeProps> = ({episode, timeline}) => {
  if (!episode || !timeline) return <AbsoluteFill style={{background: '#150f33', color: '#fff', fontSize: 60, justifyContent: 'center', alignItems: 'center'}}>No episode loaded</AbsoluteFill>;
  const t = timeline;
  const sceneAccent = (s: any) => castColor(episode.cast, s.lines[0].speaker);
  return (
    <AbsoluteFill style={{background: '#000'}}>
      <Sequence from={t.titleStart} durationInFrames={t.titleFrames}><TitleCard show={episode.show} title={episode.title} cast={episode.cast} frames={t.titleFrames} /></Sequence>
      {t.scenes.map((s: any, i: number) => (
        <Sequence key={i} from={s.start} durationInFrames={s.end - s.start}>
          <SceneView scene={s} cast={episode.cast} accent={sceneAccent(s)} />
        </Sequence>
      ))}
      <Sequence from={t.creditsStart} durationInFrames={t.creditsFrames}><Credits cast={episode.cast} title={episode.title} frames={t.creditsFrames} /></Sequence>
    </AbsoluteFill>
  );
};
