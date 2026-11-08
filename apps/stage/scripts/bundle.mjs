// Shared by render.mjs and frames.mjs: find the render browser and bundle the
// template. Every write is under a folder the caller names (the episode's work
// folder) or under the webpack cache folder, which is bounded (see CACHE_LIMIT_MB).
//   STAGE_CACHE_DIR   where the cache lives (default $XDG_CACHE_HOME/yantrik/stage, else ~/.cache/yantrik/stage)
//   STAGE_NO_CACHE=1  bundle cold and leave no cache behind (sealed or tight-disk runs)
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {bundle} from '@remotion/bundler';

const CACHE_LIMIT_MB = 500;

export const cacheRoot = () => {
  if (process.env.STAGE_CACHE_DIR) return process.env.STAGE_CACHE_DIR;
  let base = process.env.XDG_CACHE_HOME;
  if (!base) { try { base = path.join(os.homedir(), '.cache'); } catch { return null; } }
  return path.join(base, 'yantrik', 'stage');
};

const dirMegabytes = (d) => {
  let bytes = 0;
  const stack = [d];
  while (stack.length) {
    const cur = stack.pop();
    let entries = [];
    try { entries = fs.readdirSync(cur, {withFileTypes: true}); } catch { continue; }
    for (const e of entries) {
      const p = path.join(cur, e.name);
      if (e.isDirectory()) stack.push(p);
      else { try { bytes += fs.statSync(p).size; } catch { /* vanished */ } }
    }
  }
  return bytes / 1048576;
};

/** The browser Remotion fetched into node_modules/.remotion at install time (or STAGE_BROWSER). */
export function findBrowser(stage) {
  if (process.env.STAGE_BROWSER) return process.env.STAGE_BROWSER;
  const root = path.join(stage, 'node_modules', '.remotion');
  if (!fs.existsSync(root)) return null;
  const stack = [root];
  while (stack.length) {
    const d = stack.pop();
    for (const e of fs.readdirSync(d, {withFileTypes: true})) {
      const p = path.join(d, e.name);
      if (e.isDirectory()) stack.push(p);
      else if (['chrome-headless-shell', 'headless_shell', 'chrome-headless-shell.exe'].includes(e.name)) return p;
    }
  }
  return null;
}

/** A writable cache root whose size is under the limit (emptied when it is over), or null for a cold bundle. */
function usableCache() {
  if (process.env.STAGE_NO_CACHE === '1') return null;
  const rootDir = path.join(cacheRoot() ?? '', 'root');
  if (!cacheRoot()) return null;
  try {
    if (dirMegabytes(rootDir) > CACHE_LIMIT_MB) fs.rmSync(rootDir, {recursive: true, force: true});
    fs.mkdirSync(rootDir, {recursive: true});
    fs.writeFileSync(path.join(rootDir, 'package.json'), '{"name":"stage-cache","private":true}\n');
    fs.accessSync(rootDir, fs.constants.W_OK);
    return rootDir;
  } catch { return null; }   // no writable cache folder: bundle cold, slower but correct
}

/**
 * Bundle the template into outDir (the caller removes it when the film is done; it holds a copy of publicDir).
 * Remotion keeps webpack's cache at <nearest package.json>/node_modules/.cache from its root dir, so we give it
 * a root of our own in the cache folder instead of the root-owned install.
 */
export async function bundleStage(stage, publicDir, outDir) {
  const rootDir = usableCache();
  fs.rmSync(outDir, {recursive: true, force: true});
  return bundle({
    entryPoint: path.join(stage, 'src', 'index.ts'), publicDir, outDir, onProgress: () => {},
    ...(rootDir ? {rootDir, enableCaching: true} : {enableCaching: false}),
  });
}
