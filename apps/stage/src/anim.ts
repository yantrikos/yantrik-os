// Frame-driven animation for Stage: a frame context, the hooks and layout
// components the template uses, a damped spring and a piecewise-linear
// interpolation. Written from the maths (a damped harmonic oscillator pulled
// towards 1, and straight lines between breakpoints); it has no dependency
// beyond React. Sound is not here: ffmpeg mixes it from timeline.json.
import React from 'react';

export type FrameInfo = {frame: number; fps: number; width: number; height: number; durationInFrames: number};

const FrameContext = React.createContext<FrameInfo>({frame: 0, fps: 30, width: 1920, height: 1080, durationInFrames: 1});

export const FrameProvider: React.FC<{info: FrameInfo; children?: React.ReactNode}> = ({info, children}) =>
  React.createElement(FrameContext.Provider, {value: info}, children);

/** The frame of the nearest Sequence (0 at its first frame). */
export const useCurrentFrame = (): number => React.useContext(FrameContext).frame;

/** fps, size and length of the film. */
export const useVideoConfig = (): FrameInfo => React.useContext(FrameContext);

const FILL: React.CSSProperties = {
  position: 'absolute', top: 0, left: 0, right: 0, bottom: 0, width: '100%', height: '100%', display: 'flex', flexDirection: 'column',
};

/** A box that covers its parent. */
export const AbsoluteFill: React.FC<{style?: React.CSSProperties; children?: React.ReactNode}> = ({style, children}) =>
  React.createElement('div', {style: {...FILL, ...style}}, children);

/**
 * Shows its children only from frame `from` for `durationInFrames` frames (to the end when omitted),
 * and gives them a frame counted from `from`.
 */
export const Sequence: React.FC<{from?: number; durationInFrames?: number; children?: React.ReactNode}> = ({from = 0, durationInFrames = Infinity, children}) => {
  const info = React.useContext(FrameContext);
  const local = info.frame - from;
  if (local < 0 || local >= durationInFrames) return null;
  return React.createElement(FrameContext.Provider, {value: {...info, frame: local}}, React.createElement('div', {style: FILL}, children));
};

export type Extrapolate = 'extend' | 'clamp';
export type InterpolateOptions = {extrapolateLeft?: Extrapolate; extrapolateRight?: Extrapolate; easing?: (t: number) => number};

/** Map `input` along `inputRange` (strictly rising) onto `outputRange` with straight lines between points. */
export function interpolate(input: number, inputRange: readonly number[], outputRange: readonly number[], opts: InterpolateOptions = {}): number {
  const n = inputRange.length;
  if (n < 2 || n !== outputRange.length) throw new Error('interpolate: the two ranges need the same length, at least 2');
  for (let i = 1; i < n; i++) if (!(inputRange[i] > inputRange[i - 1])) throw new Error('interpolate: inputRange must be strictly increasing');
  const left = opts.extrapolateLeft ?? 'extend', right = opts.extrapolateRight ?? 'extend';
  if (input < inputRange[0] && left === 'clamp') return outputRange[0];
  if (input > inputRange[n - 1] && right === 'clamp') return outputRange[n - 1];
  let i = 1;
  while (i < n - 1 && input > inputRange[i]) i++;   // the segment [i-1, i]; the first and last extend outwards
  let t = (input - inputRange[i - 1]) / (inputRange[i] - inputRange[i - 1]);
  if (opts.easing) t = opts.easing(t);
  return outputRange[i - 1] + t * (outputRange[i] - outputRange[i - 1]);
}

export type SpringConfig = {mass: number; damping: number; stiffness: number; overshootClamping: boolean};

/**
 * Position at time t (seconds) of a unit mass-spring-damper released from rest at 0 and pulled to 1:
 * m x'' + c x' + k (x - 1) = 0. Closed form for the three damping regimes.
 */
function oscillator(t: number, {mass, damping, stiffness}: SpringConfig): number {
  if (t <= 0) return 0;
  const w0 = Math.sqrt(stiffness / mass);
  const zeta = damping / (2 * Math.sqrt(stiffness * mass));
  if (Math.abs(zeta - 1) < 1e-9) return 1 - Math.exp(-w0 * t) * (1 + w0 * t);
  if (zeta < 1) {
    const wd = w0 * Math.sqrt(1 - zeta * zeta);
    return 1 - Math.exp(-zeta * w0 * t) * (Math.cos(wd * t) + ((zeta * w0) / wd) * Math.sin(wd * t));
  }
  const s = w0 * Math.sqrt(zeta * zeta - 1);
  const r1 = -zeta * w0 + s, r2 = -zeta * w0 - s;   // both negative
  return 1 - (r2 * Math.exp(r1 * t) - r1 * Math.exp(r2 * t)) / (r2 - r1);
}

const SETTLE_BAND = 0.005;

/** Frames until the spring stays within SETTLE_BAND of 1 (searched per frame, capped at 10 s). */
function settleFrames(fps: number, config: SpringConfig): number {
  let last = 0;
  for (let f = 0; f <= fps * 10; f++) if (Math.abs(oscillator(f / fps, config) - 1) > SETTLE_BAND) last = f;
  return last + 1;
}

/** A spring from 0 to 1 over frames. With `durationInFrames` the motion is stretched so it settles in that many frames. */
export function spring({frame, fps, config = {}, durationInFrames}: {
  frame: number; fps: number; config?: Partial<SpringConfig>; durationInFrames?: number;
}): number {
  const c: SpringConfig = {mass: 1, damping: 10, stiffness: 100, overshootClamping: false, ...config};
  const stretch = durationInFrames ? settleFrames(fps, c) / durationInFrames : 1;
  const x = oscillator((frame / fps) * stretch, c);
  return c.overshootClamping ? Math.min(1, Math.max(0, x)) : x;
}
