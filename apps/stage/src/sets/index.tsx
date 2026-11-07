import React from 'react';
import {ServerRoom} from './ServerRoom';
import {BreakRoom} from './BreakRoom';
import {Rooftop} from './Rooftop';

export const SETS: Record<string, React.FC<{frame: number}>> = {
  'server-room': ServerRoom,
  'break-room': BreakRoom,
  rooftop: Rooftop,
};
