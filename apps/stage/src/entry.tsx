// The page's entry point (bundled by scripts/page.mjs). The renderer calls
// window.stage.show(...) once, then window.stage.draw(frame) per frame; each
// draw is synchronous (flushSync), and the screenshot that follows paints it.
import React from 'react';
import {createRoot} from 'react-dom/client';
import {flushSync} from 'react-dom';
import {FrameProvider} from './anim';
import {Episode} from './Episode';
import {Sheet} from './Sheet';

type Show = {kind: 'episode'; props: any; fps: number; durationInFrames: number} | {kind: 'sheet'; props: any; fps: number; durationInFrames: number};

let root: ReturnType<typeof createRoot> | null = null;
let current: Show | null = null;

(window as any).stage = {
  show(s: Show) { root = createRoot(document.getElementById('root')!); current = s; },
  draw(frame: number) {
    if (!root || !current) throw new Error('stage: show() first');
    const Comp: React.FC<any> = current.kind === 'sheet' ? Sheet : Episode;
    const info = {frame, fps: current.fps, width: 1920, height: 1080, durationInFrames: current.durationInFrames};
    flushSync(() => root!.render(<FrameProvider info={info}><Comp {...current!.props} /></FrameProvider>));
  },
};
