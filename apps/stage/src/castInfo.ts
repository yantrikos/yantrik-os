import {claudeSpec} from './rigs/Claude';
import {gptSpec} from './rigs/Gpt';
import {geminiSpec} from './rigs/Gemini';
import {grokSpec} from './rigs/Grok';

const SPECS: Record<string, {palette: {main: string; accent: string}}> = {
  CLAUDE: claudeSpec, GPT: gptSpec, GEMINI: geminiSpec, GROK: grokSpec,
};

export const ROLE: Record<string, string> = {
  CLAUDE: 'The Polite One', GPT: 'The Confident One', GEMINI: 'The Excitable One', GROK: 'The Unimpressed One',
};

/** Bright caption colour for a cast member (their palette, lifted so it reads on dark). */
export function castColor(cast: any[], name: string): string {
  const m = cast.find((c) => c.name === name);
  if (!m) return '#ffffff';
  const base = m.palette?.main ?? SPECS[m.rig].palette.main;
  const accent = SPECS[m.rig].palette.accent;
  // near-black characters (GROK) use their accent so the chip is visible
  const lum = parseInt(base.slice(1, 3), 16) * 0.3 + parseInt(base.slice(3, 5), 16) * 0.59 + parseInt(base.slice(5, 7), 16) * 0.11;
  return lum < 70 ? accent : base;
}
