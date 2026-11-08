// The staging half of the lint: props, marks, sound cues and insert shots.
// Same voice as the rest: say where, say what is allowed, offer the nearest name.
import craft from '../src/stagecraft.mjs';
import {marksOf} from '../src/stagepos.mjs';
import {closest, list, kind} from './lint-util.mjs';

export const SFX_LINE_NAMES = Object.keys(craft.sfx).filter((n) => n !== 'laugh');
export const normProp = (v) => String(v).trim().toLowerCase().replace(/[\s_]+/g, '-');
const propsOf = (set) => craft.sets[set]?.props ?? {};
const whereIs = (prop) => Object.entries(craft.sets).filter(([, s]) => prop in s.props).map(([k]) => k);

/** "alert-lamp" for the set, or null (after reporting) when it is not a prop there. */
function propName(w, raw, set, err) {
  if (typeof raw !== 'string' || !raw.trim()) { err(w, `'prop' must be the name of a prop, one of: ${list(Object.keys(propsOf(set)))}.`); return null; }
  const p = normProp(raw), mine = propsOf(set);
  if (p in mine) return p;
  const elsewhere = whereIs(p);
  const g = closest(p, Object.keys(mine));
  err(w, `prop '${raw}' is not in the ${set} set. ${set} has: ${list(Object.keys(mine))}.${elsewhere.length ? ` '${p}' lives in ${list(elsewhere)}.` : g ? ` Did you mean '${g}'?` : ''}`);
  return null;
}

/** props: [{"prop":"alert-lamp","state":"steady","text":"TRANSMIT"}] -> the same, checked and tidied. */
export function lintProps(w, v, set, {err, warn}) {
  if (v === undefined) return [];
  const arr = Array.isArray(v) ? v : v && typeof v === 'object' ? [v] : null;
  if (!arr) { err(w, `props must be a list like [{"prop":"alert-lamp","state":"steady"}]. You gave ${kind(v)}.`); return []; }
  const out = [];
  arr.forEach((o, i) => {
    const pw = `${w} props ${i + 1}`;
    if (!o || typeof o !== 'object' || Array.isArray(o)) { err(pw, `must look like {"prop":"alert-lamp","state":"steady"}. You gave ${kind(o)}.`); return; }
    for (const k of Object.keys(o)) if (!['prop', 'state', 'text'].includes(k)) warn(pw, `unknown field '${k}' is ignored. Allowed fields: prop, state, text.`);
    const prop = propName(pw, o.prop, set, err);
    if (!prop) return;
    const def = propsOf(set)[prop], one = {prop};
    if (o.state === undefined && o.text === undefined) { err(pw, `'${prop}' needs a 'state' (${list(def.states)})${def.text ? " or some 'text'" : ''}.`); return; }
    if (o.state !== undefined) {
      const s = String(o.state).trim().toLowerCase();
      if (def.states.includes(s)) one.state = s;
      else { const g = closest(s, def.states); err(pw, `state '${o.state}' is not one of: ${list(def.states)} (for ${prop}).${g ? ` Did you mean '${g}'?` : ''}`); return; }
    }
    if (o.text !== undefined) {
      if (!def.text) warn(pw, `${prop} has no words on it, so 'text' is ignored.`);
      else if (typeof o.text !== 'string') err(pw, `text must be some words like "TRANSMIT". You gave ${kind(o.text)}.`);
      else if (o.text.length > def.maxText) err(pw, `text '${o.text}' is ${o.text.length} characters; ${prop} fits ${def.maxText}. Shorten it.`);
      else one.text = o.text;
    }
    out.push(one);
  });
  return out;
}

/** move: {"GROK":"rack"} -> {GROK: "rack"}, names uppercased and marks checked against the set. */
export function lintMove(w, v, set, names, present, {err, warn}) {
  if (v === undefined) return {};
  if (!v || typeof v !== 'object' || Array.isArray(v)) { err(w, `move must look like {"GROK":"rack"}. You gave ${kind(v)}.`); return {}; }
  const marks = [...Object.keys(marksOf(set)), 'home'], out = {};
  for (const [k, m] of Object.entries(v)) {
    const nm = names.find((x) => x === k.trim().toUpperCase());
    if (!nm) { const g = closest(k.trim().toUpperCase(), names); err(w, `move name '${k}' is not in the cast (${list(names)}).${g ? ` Did you mean '${g}'?` : ''}`); continue; }
    const mark = typeof m === 'string' ? m.trim().toLowerCase() : '';
    if (!marks.includes(mark)) { const g = closest(mark, marks); err(w, `mark '${m}' is not a place in ${set}. Marks there: ${list(marks)}.${g ? ` Did you mean '${g}'?` : ''}`); continue; }
    if (!present.includes(nm)) { warn(w, `${nm} is not in this scene, so cannot walk to '${mark}'. Add ${nm} to the scene's 'present' list.`); continue; }
    out[nm] = mark;
  }
  return out;
}

/** sfx: "record-beep" or ["door","record-beep"] -> names. */
export function lintSfx(w, v, {err}) {
  if (v === undefined) return [];
  const arr = Array.isArray(v) ? v : [v], out = [];
  for (const raw of arr) {
    const s = typeof raw === 'string' ? raw.trim().toLowerCase().replace(/[\s_]+/g, '-') : '';
    if (SFX_LINE_NAMES.includes(s)) out.push(s);
    else { const g = closest(s, SFX_LINE_NAMES); err(w, `sfx '${raw}' is not one of: ${list(SFX_LINE_NAMES)}.${g ? ` Did you mean '${g}'?` : ''} (For a laugh track use "laugh": true.)`); }
  }
  return out;
}

/** camera: "insert:alert-lamp" -> normalised, or null when it is not an insert (or is wrong). */
export function lintInsert(w, raw, set, err) {
  const m = /^insert\s*:\s*(.+)$/i.exec(raw);
  if (!m) return null;
  return propName(w, m[1], set, err) ? `insert:${normProp(m[1])}` : null;
}
