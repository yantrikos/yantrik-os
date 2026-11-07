import React from 'react';
import {Composition} from 'remotion';
import {Sheet} from './Sheet';
import {Episode} from './Episode';

export const Root: React.FC = () => (
  <>
    <Composition id="Episode" component={Episode as any} width={1920} height={1080} fps={30} durationInFrames={90}
      defaultProps={{episode: null, timeline: null}}
      calculateMetadata={({props}: any) => ({durationInFrames: props.timeline?.totalFrames ?? 90})} />
    <Composition id="Sheet" component={Sheet as any} width={1920} height={1080} fps={30} durationInFrames={90}
      defaultProps={{mode: 'cast', rig: 'CLAUDE'}} />
  </>
);
