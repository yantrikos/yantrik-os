import {Claude} from './Claude';
import {Gpt} from './Gpt';
import {Gemini} from './Gemini';
import {Grok} from './Grok';

export * from './parts';
export const RIGS = {CLAUDE: Claude, GPT: Gpt, GEMINI: Gemini, GROK: Grok} as const;
export type RigName = keyof typeof RIGS;
export const RIG_NAMES = Object.keys(RIGS) as RigName[];
export {Claude, Gpt, Gemini, Grok};
