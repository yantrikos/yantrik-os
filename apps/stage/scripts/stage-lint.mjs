#!/usr/bin/env node
// Exit 0 = good (warnings are advice), 1 = problems to fix.
import fs from 'node:fs';
import path from 'node:path';
import {lintFile} from './lint.mjs';

// Usage: stage-lint.mjs episode.json [--write normalized.json] [--cache <voice cache dir>]

const args = process.argv.slice(2);
const wi = args.indexOf('--write');
let out = null;
if (wi >= 0) { out = args[wi + 1]; args.splice(wi, 2); }
const ci = args.indexOf('--cache');
let cacheDir = path.resolve('out', 'cache');   // where build.sh keeps spoken lines unless told otherwise
if (ci >= 0) { cacheDir = args[ci + 1]; args.splice(ci, 2); }
const file = args[0];
if (!file) { console.error('stage-lint: tell me which file to check, like: stage-lint episode.json'); process.exit(2); }

const r = lintFile(file, {cacheDir});
const name = path.basename(file);
const lengthWords = (r) => r.estBasis === 'voices' ? `${r.estSeconds} seconds, measured from the voices` : r.estBasis === 'mixed' ? `about ${r.estSeconds} seconds, part measured from the voices` : `about ${r.estSeconds} seconds, a guess from the word count. build.sh reports the true length once the voices are made`;
if (r.errors.length) {
  console.log(`stage-lint: ${r.errors.length} problem${r.errors.length === 1 ? '' : 's'} in ${name}. Fix ${r.errors.length === 1 ? 'it' : 'them'} and run again.`);
  r.errors.forEach((e, i) => console.log(`  ${i + 1}. ${e}`));
}
if (r.warnings.length) {
  if (!r.errors.length) console.log(`stage-lint: ${name} is good, with ${r.warnings.length} note${r.warnings.length === 1 ? '' : 's'}.`);
  r.warnings.forEach((w) => console.log(`  note: ${w}`));
  if (!r.errors.length) console.log(`  length: ${lengthWords(r)}.`);
}
if (!r.errors.length) {
  if (!r.warnings.length) console.log(`stage-lint: ${name} is good (${lengthWords(r)}).`);
  if (out) { fs.mkdirSync(path.dirname(out), {recursive: true}); fs.writeFileSync(out, JSON.stringify(r.episode, null, 2) + '\n'); }
}
process.exit(r.errors.length ? 1 : 0);
