// What state is each prop of a set in at frame g? Starts from the scene's
// carried-over state, then applies the episode's changes in order.
import craft from '../stagecraft.mjs';

export type PropView = {state: string; text: string; since: number};
export type PropViews = Record<string, PropView>;
export type PropArt = React.FC<PropView & {frame: number}>;

export function propsAt(scene: any, g: number): PropViews {
  const defs = (craft.sets as any)[scene.set]?.props ?? {};
  const out: PropViews = {};
  for (const [name, d] of Object.entries<any>(defs)) {
    const s = scene.propsStart?.[name];
    out[name] = {state: s?.state ?? d.default, text: s?.text ?? d.defaultText ?? '', since: scene.start};
  }
  for (const e of scene.propEvents ?? []) {
    if (e.at > g || !out[e.prop]) continue;
    out[e.prop] = {state: e.state ?? out[e.prop].state, text: e.text ?? out[e.prop].text, since: e.at};
  }
  return out;
}

export const insertOf = (set: string, prop: string): {cx: number; cy: number; z: number} | null => (craft.sets as any)[set]?.props?.[prop]?.insert ?? null;
