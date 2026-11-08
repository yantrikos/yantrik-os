// The director: decides where the camera is for every moment of a scene.
// An episode that names a camera gets exactly that. One that leaves it out
// ("auto") gets coverage: establish wide, work in two-shots, go close on a big
// reaction or a short line, cut to a bystander who is shocked or laughing,
// and push in on the last line.
import type {ShotKind} from './layout';

export type Seg = {from: number; kind: ShotKind | 'insert'; subject: number; partner: number; prop?: string; runFrom: number; runTo: number};

const BIG = ['shocked', 'angry', 'sad', 'laughing', 'confused'];
const BYSTANDER = ['shocked', 'laughing', 'sad', 'angry'];

export function planShots(scene: any): Seg[] {
  const names: string[] = scene.present;
  const n = names.length;
  const lines: any[] = scene.lines;
  const idx = (nm: string) => Math.max(0, names.indexOf(nm));
  const partnerOf = (li: number) => {
    const me = idx(lines[li].speaker);
    const prev = lines[li - 1];
    if (prev && prev.speaker !== lines[li].speaker && names.includes(prev.speaker)) return idx(prev.speaker);
    const next = lines.slice(li + 1).find((l) => l.speaker !== lines[li].speaker && names.includes(l.speaker));
    if (next) return idx(next.speaker);
    return me === 0 ? Math.min(1, n - 1) : me - 1;
  };
  const segs: Seg[] = [];
  const add = (from: number, kind: Seg['kind'], subject: number, partner: number, prop?: string) =>
    segs.push({from, kind, subject, partner, prop, runFrom: from, runTo: from});
  let last: Seg['kind'] = 'wide', sinceWide = 0;

  lines.forEach((l, li) => {
    const subject = idx(l.speaker), partner = partnerOf(li);
    const cam: string | null = l.camera ?? (scene.camera !== 'auto' ? scene.camera : null);
    if (cam && cam.startsWith('insert:')) { add(l.begin, 'insert', subject, partner, cam.slice(7)); last = 'insert'; return; }
    let kind: ShotKind;
    if (cam && cam !== 'auto') kind = cam as ShotKind;
    else if (Object.keys(l.move ?? {}).length) kind = 'wide';
    else if (li === 0) kind = n >= 3 ? 'wide' : 'two-shot';
    else if (li === lines.length - 1) kind = 'push-in';
    else if ((BIG.includes(l.emotion) || l.text.split(/\s+/).length <= 5) && last !== 'close-up') kind = 'close-up';
    else if (n >= 3 && sinceWide >= 3) kind = 'wide';
    else kind = 'two-shot';
    sinceWide = kind === 'wide' ? 0 : sinceWide + 1;
    add(l.begin, kind, subject, partner);
    last = kind;
    // cut to a bystander who is reacting while this line plays out (auto coverage only)
    if (!cam || cam === 'auto') {
      const r = Object.entries(l.react ?? {}).find(([nm, em]) => nm !== l.speaker && names.includes(nm) && BYSTANDER.includes(em as string));
      if (r && l.audioFrames >= 40 && kind !== 'close-up') {
        add(l.start + Math.round(l.audioFrames * 0.6), 'close-up', idx(r[0]), subject);
        last = 'close-up';
      }
    }
  });
  // extents of each run of one kind of shot, so a push-in creeps over the whole run
  for (let i = 0; i < segs.length; i++) {
    let a = i, b = i;
    while (a > 0 && segs[a - 1].kind === segs[i].kind) a--;
    while (b < segs.length - 1 && segs[b + 1].kind === segs[i].kind) b++;
    segs[i].runFrom = segs[a].from;
    segs[i].runTo = b < segs.length - 1 ? segs[b + 1].from : scene.end;
  }
  return segs;
}
