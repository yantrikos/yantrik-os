// The voice cache: one spoken line is stored under a key made from its text
// and its voice settings. voices.mjs writes it; the lint reads it, so the
// length it reports for a re-run is the length of the real voices.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import {fileURLToPath} from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
export const loadBank = () => JSON.parse(fs.readFileSync(path.join(here, '..', 'voices', 'voices.json'), 'utf8'));

export function cacheKey(bank, text, voiceKey) {
  const v = bank.voices[voiceKey];
  return crypto.createHash('sha1').update(JSON.stringify([bank.version, text, v])).digest('hex').slice(0, 16);
}

/** Seconds of a line already spoken into `cacheDir`, or null. */
export function cachedSeconds(cacheDir, bank, text, voiceKey) {
  if (!cacheDir || !bank.voices[voiceKey]) return null;
  try {
    const m = JSON.parse(fs.readFileSync(path.join(cacheDir, `${cacheKey(bank, text, voiceKey)}.json`), 'utf8'));
    return typeof m.durationSec === 'number' ? m.durationSec : null;
  } catch { return null; }
}
