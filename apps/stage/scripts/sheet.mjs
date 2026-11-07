// QA: node scripts/sheet.mjs <mode> <rig> <out.png>  (cast|expressions|visemes|poses)
import {bundle} from '@remotion/bundler';
import {renderStill, selectComposition} from '@remotion/renderer';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const [mode, rig, out, frame] = process.argv.slice(2);
const serveUrl = await bundle({entryPoint: path.join(root, 'src/index.ts')});
const inputProps = {mode, rig};
const comp = await selectComposition({serveUrl, id: 'Sheet', inputProps});
await renderStill({composition: comp, serveUrl, output: out, inputProps, frame: Number(frame || 20)});
