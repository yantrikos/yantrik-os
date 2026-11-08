// Shared by render.mjs and frames.mjs: find the render browser and bundle the
// template with webpack's cache kept somewhere the person can write
// ($XDG_CACHE_HOME/yantrik/stage), never inside the root-owned install.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {bundle} from '@remotion/bundler';

export const cacheRoot = () => path.join(process.env.XDG_CACHE_HOME || path.join(os.homedir(), '.cache'), 'yantrik', 'stage');

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

export async function bundleStage(stage, publicDir) {
  const cacheDirectory = path.join(cacheRoot(), 'webpack');
  let cache = false;
  try { fs.mkdirSync(cacheDirectory, {recursive: true}); fs.accessSync(cacheDirectory, fs.constants.W_OK); cache = {type: 'filesystem', cacheDirectory, name: 'stage'}; }
  catch { /* no writable cache: bundle cold, slower but correct */ }
  return bundle({
    entryPoint: path.join(stage, 'src', 'index.ts'), publicDir, onProgress: () => {},
    enableCaching: false,
    webpackOverride: (c) => ({...c, cache}),
  });
}
