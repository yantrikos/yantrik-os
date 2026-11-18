// Small helpers the lint sentences are built from: nearest-name suggestions and plain descriptions of a wrong value.
export const lev = (a, b) => {
  const m = Array.from({length: a.length + 1}, (_, i) => [i, ...Array(b.length).fill(0)]);
  for (let j = 1; j <= b.length; j++) m[0][j] = j;
  for (let i = 1; i <= a.length; i++)
    for (let j = 1; j <= b.length; j++)
      m[i][j] = Math.min(m[i - 1][j] + 1, m[i][j - 1] + 1, m[i - 1][j - 1] + (a[i - 1] === b[j - 1] ? 0 : 1));
  return m[a.length][b.length];
};
export const closest = (v, opts) => {
  const s = String(v).toLowerCase();
  let best = null, bd = 99;
  for (const o of opts) {
    const d = lev(s, String(o).toLowerCase());
    if (d < bd) { bd = d; best = o; }
  }
  return bd <= Math.max(2, Math.floor(String(v).length / 2)) ? best : null;
};
export const list = (a) => a.join(', ');
export const kind = (v) => (v === null ? 'null' : Array.isArray(v) ? 'a list' : typeof v === 'object' ? 'an object' : `a ${typeof v}`);
