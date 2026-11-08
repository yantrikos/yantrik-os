// stage-lint core: validates an episode against episode.schema.json and
// answers in plain sentences naming the exact place and the allowed values.
// It also returns a normalised episode with every default filled in, which
// the voice and render steps consume. Case and stray spaces are forgiven.
import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {closest, list, kind} from './lint-util.mjs';
import {lintProps, lintMove, lintSfx, lintInsert} from './lint-stage.mjs';
import {layoutTimeline, FPS, WORDS_PER_SECOND} from './timeline.mjs';
import {loadBank, cachedSeconds} from './voicecache.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
export const schema = JSON.parse(fs.readFileSync(path.join(here, '..', 'episode.schema.json'), 'utf8'));
const D = schema.$defs;
const ENUM = {
  rig: D.castMember.properties.rig.enum,
  voice: D.castMember.properties.voice.enum,
  cue: D.music.properties.cue.enum,
  set: D.scene.properties.set.enum,
  camera: D.scene.properties.camera.enum,
  transition: D.scene.properties.transition.enum,
  emotion: D.line.properties.emotion.enum,
  pose: D.line.properties.pose.enum,
  part: D.scene.properties.part.enum,
};
const ACCESSORIES = schema['x-accessories'];
const DEFAULT_VOICE = schema['x-default-voice'];
const HEX = /^#[0-9a-fA-F]{6}$/;

// Every size limit comes from the schema, so the lint and the schema cannot drift apart.
const P = schema.properties;
const LIM = {
  ...schema['x-limits'],
  title: P.title.maxLength, show: P.show.maxLength, cast: P.cast.maxItems, scenes: P.scenes.maxItems,
  lines: D.scene.properties.lines.maxItems, text: D.line.properties.text.maxLength,
  hold: D.line.properties.hold.maximum, pause: D.line.properties.pause.maximum,
};
const LONG_TEXT = 170;   // a long speech for a sitcom: advice, not an error

/** opts.cacheDir: a Stage voice cache; lines already spoken there count at their true length. */
export function lintEpisode(ep, opts = {}) {
  const errors = [], warnings = [];
  const err = (where, msg) => errors.push(`${where}: ${msg}`);
  const warn = (where, msg) => warnings.push(`${where}: ${msg}`);

  const pick = (where, field, v, allowed, dflt, norm) => {
    if (v === undefined || v === null || v === '') return dflt;
    if (typeof v !== 'string') { err(where, `${field} must be one of: ${list(allowed)}. You gave ${kind(v)}.`); return dflt; }
    const t = v.trim();
    const hit = allowed.find((a) => a.toLowerCase() === t.toLowerCase());
    if (hit) return hit;
    const g = closest(t, allowed);
    err(where, `${field} '${v}' is not one of: ${list(allowed)}.${g ? ` Did you mean '${g}'?` : ''}`);
    return dflt;
  };
  const unknownKeys = (where, obj, allowed) => {
    for (const k of Object.keys(obj)) {
      if (allowed.includes(k)) continue;
      const g = closest(k, allowed);
      err(where, `unknown field '${k}'.${g ? ` Did you mean '${g}'?` : ''} Allowed fields: ${list(allowed)}.`);
    }
  };

  if (ep === null || typeof ep !== 'object' || Array.isArray(ep)) {
    return {errors: [`episode: the file must hold one JSON object with 'title' and 'scenes', but it holds ${kind(ep)}.`], warnings, episode: null};
  }
  unknownKeys('episode', ep, Object.keys(schema.properties));

  // title
  let title = ep.title;
  if (typeof title !== 'string' || !title.trim()) { err('title',
    `must be some text, like "The Server Room Incident". ${title === undefined ? "It is missing." : `You gave ${kind(title)}.`}`); title = 'Untitled'; }
  else if (title.length > LIM.title) { err('title', `is ${title.length} characters long; the limit is ${LIM.title}. Shorten it.`); }
  let show = 'THE CLUSTER';
  if (ep.show !== undefined) {
    if (typeof ep.show !== 'string' || !ep.show.trim()) err('show', `must be some text like "THE CLUSTER". You gave ${kind(ep.show)}.`);
    else if (ep.show.trim().length > LIM.show) err('show', `is ${ep.show.trim().length} characters long; the limit is ${LIM.show}. Shorten it.`);
    else show = ep.show.trim();
  }

  // cast
  let castIn = ep.cast;
  if (castIn === undefined) castIn = ['CLAUDE', 'GPT', 'GEMINI', 'GROK'].map((n) => ({name: n}));
  const cast = [];
  if (!Array.isArray(castIn) || castIn.length === 0) err('cast', `must be a list of 1 to 4 characters like [{"name":"CLAUDE","rig":"CLAUDE"}]. You gave ${kind(castIn)}. Leave 'cast' out to get all four stars.`);
  else {
    if (castIn.length > LIM.cast) err('cast', `has ${castIn.length} characters; the limit is ${LIM.cast}.`);
    castIn.slice(0, LIM.cast).forEach((m, i) => {
      const w = `cast ${i + 1}`;
      if (!m || typeof m !== 'object' || Array.isArray(m)) { err(w, `must be an object like {"name":"CLAUDE","rig":"CLAUDE"}. You gave ${kind(m)}.`); return; }
      unknownKeys(w, m, Object.keys(D.castMember.properties));
      let name = typeof m.name === 'string' ? m.name.trim().toUpperCase() : '';
      if (!name) { err(w, `'name' is missing. Give a short UPPERCASE name such as "CLAUDE".`); name = `CAST${i + 1}`; }
      else if (!/^[A-Z][A-Z0-9_ ]{0,15}$/.test(name)) err(w, `name '${m.name}' must be 1 to 16 letters, digits or spaces, starting with a letter, like "CLAUDE".`);
      if (cast.some((c) => c.name === name)) err(w, `name '${name}' is used twice. Every cast name must be different.`);
      const dflt = ENUM.rig.includes(name) ? name : ENUM.rig[i % 4];
      const rig = pick(w, 'rig', m.rig, ENUM.rig, dflt);
      const voice = pick(w, 'voice', m.voice, ENUM.voice, DEFAULT_VOICE[rig]);
      const palette = {};
      if (m.palette !== undefined) {
        if (!m.palette || typeof m.palette !== 'object' || Array.isArray(m.palette)) err(w, `palette must look like {"main":"#d9774f"}. You gave ${kind(m.palette)}.`);
        else {
          unknownKeys(`${w} palette`, m.palette, ['main', 'accent']);
          for (const k of ['main', 'accent']) {
            if (m.palette[k] === undefined) continue;
            if (typeof m.palette[k] === 'string' && HEX.test(m.palette[k])) palette[k] = m.palette[k];
            else err(w, `palette ${k} '${m.palette[k]}' must be a hex colour like "#d9774f" (six digits).`);
          }
        }
      }
      let accessory;
      if (m.accessory !== undefined) {
        const allowed = [...ACCESSORIES[rig], 'none'];
        accessory = pick(w, `accessory (for ${rig})`, m.accessory, allowed, undefined);
      }
      cast.push({name, rig, voice, palette, accessory});
    });
  }
  const names = cast.map((c) => c.name);

  // music
  const music = {cue: 'cheeky', volume: 0.5};
  if (ep.music !== undefined) {
    if (!ep.music || typeof ep.music !== 'object' || Array.isArray(ep.music)) err('music', `must look like {"cue":"cheeky","volume":0.5}. You gave ${kind(ep.music)}.`);
    else {
      unknownKeys('music', ep.music, ['cue', 'volume']);
      music.cue = pick('music', 'cue', ep.music.cue, ENUM.cue, 'cheeky');
      if (ep.music.volume !== undefined) {
        const v = ep.music.volume;
        if (typeof v !== 'number' || v < 0 || v > 1) err('music', `volume must be a number from 0 to 1. You gave ${JSON.stringify(v)}.`);
        else music.volume = v;
      }
    }
  }

  // scenes
  const scenes = [];
  if (!Array.isArray(ep.scenes) || ep.scenes.length === 0) err('scenes', `must be a list of 1 to ${LIM.scenes} scenes, each with a 'lines' list. ${ep.scenes === undefined ? 'It is missing.' : `You gave ${kind(ep.scenes)}.`}`);
  else {
    if (ep.scenes.length > LIM.scenes) err('scenes', `has ${ep.scenes.length} scenes; the limit is ${LIM.scenes}.`);
    ep.scenes.slice(0, LIM.scenes).forEach((s, si) => {
      const sw = `scene ${si + 1}`;
      if (!s || typeof s !== 'object' || Array.isArray(s)) { err(sw, `must be an object with a 'lines' list. You gave ${kind(s)}.`); return; }
      unknownKeys(sw, s, Object.keys(D.scene.properties));
      const set = pick(sw, 'set', s.set, ENUM.set, 'server-room');
      const camera = pick(sw, 'camera', s.camera, ENUM.camera, 'auto');
      const part = pick(sw, 'part', s.part, ENUM.part, 'main');
      const firstOfPart = !ep.scenes.slice(0, si).some((x) => x && typeof x === 'object' && (x.part ?? 'main') === part) && part !== 'tag';
      const transition = pick(sw, 'transition', s.transition, ENUM.transition, firstOfPart ? 'cut' : 'fade');
      const sprops = lintProps(sw, s.props, set, {err, warn}, LIM.maxPropsPerList);
      let present = names.slice();
      if (s.present !== undefined) {
        if (!Array.isArray(s.present) || s.present.length === 0) err(sw, `'present' must be a list of cast names like ["CLAUDE","GPT"]. You gave ${kind(s.present)}.`);
        else {
          present = [];
          for (const n of s.present) {
            const hit = names.find((x) => x === String(n).trim().toUpperCase());
            if (hit) { if (!present.includes(hit)) present.push(hit); }
            else { const g = closest(String(n), names); err(sw, `present name '${n}' is not in the cast (${list(names)}).${g ? ` Did you mean '${g}'?` : ''}`); }
          }
        }
      }
      const lines = [];
      if (!Array.isArray(s.lines) || s.lines.length === 0) err(sw, `'lines' must be a list of at least one line like {"speaker":"CLAUDE","text":"Hello."}. ${s.lines === undefined ? 'It is missing.' : `You gave ${kind(s.lines)}.`}`);
      else {
        if (s.lines.length > LIM.lines) err(sw, `has ${s.lines.length} lines; the limit is ${LIM.lines}. Split it into two scenes.`);
        s.lines.slice(0, LIM.lines).forEach((l, li) => {
          const lw = `${sw} line ${li + 1}`;
          if (!l || typeof l !== 'object' || Array.isArray(l)) { err(lw, `must be an object like {"speaker":"CLAUDE","text":"Hello."}. You gave ${kind(l)}.`); return; }
          unknownKeys(lw, l, Object.keys(D.line.properties));
          let speaker = typeof l.speaker === 'string' ? l.speaker.trim().toUpperCase() : '';
          if (!speaker) err(lw, `'speaker' is missing. Use one of the cast names: ${list(names)}.`);
          else if (!names.includes(speaker)) { const g = closest(speaker, names); err(lw, `speaker '${l.speaker}' is not in the cast (${list(names)}).${g ? ` Did you mean '${g}'?` : ''}`); }
          else if (!present.includes(speaker)) { warn(lw, `${speaker} speaks but is not in this scene's 'present' list, so they are added to the scene.`); present.push(speaker); }
          let text = l.text;
          if (typeof text !== 'string' || !text.trim()) { err(lw, `'text' is missing. Every line needs the words the character says.`); text = ''; }
          else if (text.length > LIM.text) err(lw, `text is ${text.length} characters long; the limit is ${LIM.text}. Split it into two lines.`);
          else if (text.length > LONG_TEXT) warn(lw, `text is ${text.length} characters, which is a long speech for a sitcom. Shorter lines are funnier.`);
          const emotion = pick(lw, 'emotion', l.emotion, ENUM.emotion, 'neutral');
          const pose = pick(lw, 'pose', l.pose, ENUM.pose, undefined);
          let lcamera;
          if (typeof l.camera === 'string' && /^insert\s*:/i.test(l.camera)) lcamera = lintInsert(lw, l.camera, set, err) ?? undefined;
          else lcamera = pick(lw, 'camera', l.camera, ENUM.camera, undefined);
          let hold = 0;
          if (l.hold !== undefined) {
            if (typeof l.hold !== 'number' || l.hold < 0 || l.hold > LIM.hold) err(lw, `hold must be a number of seconds from 0 to ${LIM.hold}. You gave ${JSON.stringify(l.hold)}.`);
            else hold = l.hold;
          }
          const lprops = lintProps(lw, l.props, set, {err, warn}, LIM.maxPropsPerList);
          const move = lintMove(lw, l.move, set, names, present, {err, warn});
          const sfx = lintSfx(lw, l.sfx, {err}, LIM.maxSfxPerLine);
          if (l.laugh !== undefined && typeof l.laugh !== 'boolean') err(lw, `laugh must be true or false. You gave ${JSON.stringify(l.laugh)}.`);
          let pause = 0.25;
          if (l.pause !== undefined) {
            if (typeof l.pause !== 'number' || l.pause < 0 || l.pause > LIM.pause) err(lw, `pause must be a number of seconds from 0 to ${LIM.pause}. You gave ${JSON.stringify(l.pause)}.`);
            else pause = l.pause;
          }
          const react = {};
          if (l.react !== undefined) {
            if (!l.react || typeof l.react !== 'object' || Array.isArray(l.react)) err(lw, `react must look like {"GROK":"smug"}. You gave ${kind(l.react)}.`);
            else for (const [k, v] of Object.entries(l.react)) {
              const nm = names.find((x) => x === k.trim().toUpperCase());
              if (!nm) { err(lw, `react name '${k}' is not in the cast (${list(names)}).`); continue; }
              react[nm] = pick(lw, `react ${nm}`, v, ENUM.emotion, 'neutral');
            }
          }
          lines.push({speaker, text: String(text).trim(), emotion, pose, camera: lcamera, pause, hold, react, props: lprops, move, sfx, laugh: l.laugh === true});
        });
      }
      if (lines.length >= 3 && lines.every((x) => !x.hold)) {
        const last = lines[lines.length - 1];
        warn(sw, `the last line (${last.speaker}: "${last.text.slice(0, 28)}${last.text.length > 28 ? '...' : ''}") has no hold. A beat of silence before a punchline lands it: add "hold": 0.6 to that line.`);
      }
      scenes.push({set, camera, transition, part, present, props: sprops, lines});
    });
  }
  const totalLines = scenes.reduce((n, s) => n + s.lines.length, 0);
  if (totalLines > LIM.maxTotalLines) err('episode', `has ${totalLines} lines in all; the limit is ${LIM.maxTotalLines}. Cut the weakest scenes or lines.`);
  let est = 0, basis = 'words';
  let episode = errors.length ? null : {title: String(title).trim(), show, cast, music, scenes};
  if (episode) {
    const bank = loadBank(), voiceOf = Object.fromEntries(cast.map((c) => [c.name, c.voice]));
    const heard = [];   // [words, seconds] of lines already spoken
    const words = (t) => t.split(/\s+/).length;
    const known = new Map();
    for (const sc of scenes) for (const l of sc.lines) {
      const sec = cachedSeconds(opts.cacheDir, bank, l.text, voiceOf[l.speaker]);
      if (sec !== null) { known.set(l, sec); heard.push([words(l.text), sec]); }
    }
    const pace = heard.length >= 3 ? heard.reduce((a, h) => a + h[0], 0) / heard.reduce((a, h) => a + h[1], 0) : WORDS_PER_SECOND;
    const tl = layoutTimeline(episode, (l) => known.get(l) ?? words(l.text) / pace);
    est = Math.round(tl.totalFrames / FPS);
    basis = known.size === 0 ? 'words' : known.size === heard.length && heard.length === scenes.reduce((n, s) => n + s.lines.length, 0) ? 'voices' : 'mixed';
    if (est < 30) warn('episode', `runs about ${est} seconds. A good episode is 45 to 60 seconds; add a scene or more lines.`);
    else if (est > LIM.maxSeconds) { err('episode', `runs about ${est} seconds; the limit is ${LIM.maxSeconds}. A good episode is 45 to 60 seconds; cut scenes or lines.`); episode = null; }
    else if (est > 120) warn('episode', `runs about ${est} seconds. A good episode is 45 to 60 seconds; trim some lines.`);
  }
  return {errors, warnings, estSeconds: est, estBasis: basis, episode};
}

export function lintFile(file, opts = {}) {
  let raw;
  try {
    const size = fs.statSync(file).size;
    if (size > LIM.maxFileBytes) return {errors: [`file: '${path.basename(file)}' is ${size} bytes; the limit is ${LIM.maxFileBytes}. An episode is a short script, not a book.`],
      warnings: [], episode: null};
    raw = fs.readFileSync(file, 'utf8');
  }
  catch { return {errors: [`file: I cannot read '${file}'. Check the path and write episode.json first.`], warnings: [], episode: null}; }
  raw = raw.replace(/^﻿/, '');
  let obj;
  try { obj = JSON.parse(raw); }
  catch (e) {
    const m = /position (\d+)/.exec(e.message);
    let where = '';
    if (m) { const pos = Number(m[1]); const before = raw.slice(0, pos); where = ` near line ${before.split('\n').length}, column ${pos - before.lastIndexOf('\n')}`; }
    return {errors: [`file: '${path.basename(file)}' is not valid JSON${where}: ${e.message.split('\n')[0]}. Common causes: a missing comma, a trailing comma, or a double quote inside text that needs a backslash.`],
      warnings: [], episode: null};
  }
  return lintEpisode(obj, opts);
}
