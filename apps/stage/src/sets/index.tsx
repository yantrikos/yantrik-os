import React from 'react';
import type {PropViews} from '../props/state';
import {ServerRoom} from './ServerRoom';
import {BreakRoom} from './BreakRoom';
import {Rooftop} from './Rooftop';

export const SETS: Record<string, React.FC<{frame: number; props: PropViews}>> = {
  'server-room': ServerRoom,
  'break-room': BreakRoom,
  rooftop: Rooftop,
};
