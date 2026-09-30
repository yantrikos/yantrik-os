// The page, as the browser surface reads it. Evaluated in an isolated world of the top frame:
// it shares the page's DOM and none of its JavaScript, so a page can neither see this registry
// nor rewrite it, and a page's own `Element.prototype` patches do not reach it.
//
// What it gives the surface, on globalThis.__yb:
//   snapshot(all)      the things a person could act on, and the headings, alerts and dialogs
//                      that say where they are — through open shadow roots and same-origin
//                      frames, each with a ref that stays the same for as long as the element
//                      exists, its role, name, state and box.
//   find(q)            where the thing wanted is, anywhere on the page.
//   text(limit)        what the page says, as a reader sees it.
//   target(ref)        scroll one element into view and say where to press it — or what covers it.
//   focus/clear/pick   the small moves typing and choosing need.
//   watch()/changes()  what appeared on the page between an action and its settling.
(() => {
  // A page can make `globalThis.__yb` exist (an element with id="__yb" is a named property of
  // the window, in every world), so the registry is recognised by a mark only it carries.
  if (globalThis.__yb && globalThis.__yb.__yantrik === true) return 'ready';

  // `document.title` and `document.activeElement` can be shadowed by the page: an <img
  // name="title"> is what `document.title` reads, on every world. The prototype's getters cannot.
  const DOC_TITLE = Object.getOwnPropertyDescriptor(Document.prototype, 'title').get;
  const DOC_ACTIVE = Object.getOwnPropertyDescriptor(Document.prototype, 'activeElement').get;
  const titleOf = (doc) => String(DOC_TITLE.call(doc) || '');
  const activeIn = (doc) => DOC_ACTIVE.call(doc);

  // ── refs: one per element, for the element's lifetime ──
  const ids = new WeakMap();
  const refs = new Map();
  let next = 1;
  const refOf = (el) => {
    let id = ids.get(el);
    if (!id) {
      id = 'e' + next++;
      ids.set(el, id);
      refs.set(id, new WeakRef(el));
    }
    return id;
  };
  const byRef = (id) => {
    const held = refs.get(id);
    const el = held && held.deref();
    return el && el.isConnected ? el : null;
  };

  // ── walking the composed tree: open shadow roots and same-origin frames ──
  // Yields [element, frame] where frame = {win, ox, oy, depth, origin}: the window the element
  // lives in and that window's offset in the top frame's viewport.
  function* walk(root, frame) {
    const stack = [root];
    while (stack.length) {
      const node = stack.pop();
      const kids = node.children ? Array.from(node.children) : [];
      for (let i = kids.length - 1; i >= 0; i--) stack.push(kids[i]);
      if (node.nodeType !== 1) continue;
      yield [node, frame];
      if (node.shadowRoot) {
        const inner = Array.from(node.shadowRoot.children);
        for (let i = inner.length - 1; i >= 0; i--) stack.push(inner[i]);
      }
      if (node.tagName === 'IFRAME' || node.tagName === 'FRAME') {
        let doc = null;
        try { doc = node.contentDocument; } catch (e) { doc = null; }
        const r = node.getBoundingClientRect();
        const sub = {
          win: node.contentWindow,
          ox: frame.ox + r.left + node.clientLeft,
          oy: frame.oy + r.top + node.clientTop,
          depth: frame.depth + 1,
          el: node,
        };
        if (doc && doc.documentElement) {
          yield* walk(doc.documentElement, sub);
        } else {
          blindFrames.push(node);
        }
      }
    }
  }
  let blindFrames = [];
  const TOP = { win: window, ox: 0, oy: 0, depth: 0, el: null };

  // ── what an element is ──
  const INTERACTIVE_ROLES = new Set(['button', 'link', 'tab', 'menuitem', 'menuitemcheckbox',
    'menuitemradio', 'checkbox', 'radio', 'switch', 'option', 'combobox', 'textbox', 'searchbox',
    'slider', 'spinbutton', 'treeitem', 'listbox']);
  const CONTEXT_ROLES = new Set(['heading', 'alert', 'alertdialog', 'dialog', 'status']);

  const roleOf = (el) => {
    const explicit = (el.getAttribute('role') || '').trim().split(/\s+/)[0];
    if (explicit) return explicit;
    const tag = el.tagName;
    if (tag === 'A') return el.hasAttribute('href') ? 'link' : '';
    if (tag === 'BUTTON' || tag === 'SUMMARY') return 'button';
    if (tag === 'TEXTAREA') return 'textbox';
    if (tag === 'SELECT') return el.multiple ? 'listbox' : 'combobox';
    if (/^H[1-6]$/.test(tag)) return 'heading';
    if (tag === 'DIALOG') return 'dialog';
    if (tag === 'INPUT') {
      const t = (el.getAttribute('type') || 'text').toLowerCase();
      if (t === 'hidden') return '';
      if (['submit', 'button', 'reset', 'image'].includes(t)) return 'button';
      if (t === 'checkbox') return 'checkbox';
      if (t === 'radio') return 'radio';
      if (t === 'range') return 'slider';
      if (t === 'number') return 'spinbutton';
      if (t === 'search') return 'searchbox';
      return 'textbox';
    }
    if (el.isContentEditable && (el.getAttribute('contenteditable') !== null)) return 'textbox';
    return '';
  };

  const clean = (s, n) => {
    s = (s || '').replace(/\s+/g, ' ').trim();
    return s.length > n ? s.slice(0, n - 1) + '…' : s;
  };

  const textOf = (el) => clean(el.innerText !== undefined ? el.innerText : el.textContent, 120);

  const nameOf = (el) => {
    const doc = el.ownerDocument;
    const by = el.getAttribute('aria-labelledby');
    if (by) {
      const t = by.split(/\s+/).map((id) => {
        const n = doc.getElementById(id);
        return n ? textOf(n) : '';
      }).join(' ');
      if (t.trim()) return clean(t, 100);
    }
    const aria = el.getAttribute('aria-label');
    if (aria && aria.trim()) return clean(aria, 100);
    const tag = el.tagName;
    if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') {
      if (el.id) {
        const lab = doc.querySelector('label[for="' + CSS.escape(el.id) + '"]');
        if (lab && textOf(lab)) return clean(textOf(lab), 100);
      }
      const wrap = el.closest('label');
      if (wrap && textOf(wrap)) return clean(textOf(wrap), 100);
      const t = (el.getAttribute('type') || '').toLowerCase();
      if (['submit', 'button', 'reset'].includes(t) && el.value) return clean(el.value, 100);
      if (t === 'image' && el.getAttribute('alt')) return clean(el.getAttribute('alt'), 100);
      if (t === 'submit' || t === 'image') return 'Submit';
      if (t === 'reset') return 'Reset';
      const ph = el.getAttribute('placeholder') || el.getAttribute('title') || el.getAttribute('name');
      if (ph) return clean(ph, 100);
      return '';
    }
    const own = textOf(el);
    if (own) return clean(own, 100);
    const img = el.querySelector && el.querySelector('img[alt], svg[aria-label], [title]');
    if (img) return clean(img.getAttribute('alt') || img.getAttribute('aria-label') || img.getAttribute('title'), 100);
    return clean(el.getAttribute('title') || '', 100);
  };

  const hidden = (el, win) => {
    if (el.closest('[aria-hidden="true"]') && !el.closest('[aria-modal="true"]')) return true;
    const style = win.getComputedStyle(el);
    if (style.display === 'none' || style.visibility === 'hidden' || style.visibility === 'collapse') return true;
    if (parseFloat(style.opacity) === 0) return true;
    const r = el.getBoundingClientRect();
    return r.width < 1 || r.height < 1;
  };

  // A div a site made clickable by hand: the pointer says so, and nothing clickable holds it.
  // Asking every div for its computed style is what made a long page slow to read (a Hacker
  // News thread is thousands of spans), so the cheap tests come first and only what is in view
  // is asked: a hand-made button off screen is found by scrolling to it, like one in view.
  const HANDMADE_TAGS = new Set(['DIV', 'SPAN', 'LI', 'IMG', 'svg', 'TD']);
  const handmade = (el, win) => {
    if (!HANDMADE_TAGS.has(el.tagName)) return false;
    if (win.getComputedStyle(el).cursor !== 'pointer') return false;
    const parent = el.parentElement;
    if (parent && win.getComputedStyle(parent).cursor === 'pointer') return false;
    return !el.closest('a[href],button,[role="button"],[role="link"],label,summary');
  };

  const stateOf = (el, role) => {
    const s = {};
    if (el.disabled || el.getAttribute('aria-disabled') === 'true') s.disabled = true;
    if (role === 'checkbox' || role === 'radio' || role === 'switch' || role === 'menuitemcheckbox') {
      s.checked = el.checked !== undefined ? !!el.checked : el.getAttribute('aria-checked') === 'true';
    }
    const exp = el.getAttribute('aria-expanded');
    if (exp !== null) s.expanded = exp === 'true';
    if (el.getAttribute('aria-selected') === 'true') s.selected = true;
    if (el.required || el.getAttribute('aria-required') === 'true') s.required = true;
    if (el.getAttribute('aria-invalid') === 'true') s.invalid = true;
    if (el.readOnly) s.readonly = true;
    if (el.tagName === 'SELECT') {
      const o = el.options[el.selectedIndex];
      if (o) s.value = clean(o.text, 60);
      s.options = Array.from(el.options).slice(0, 12).map((o) => clean(o.text, 40));
      if (el.options.length > 12) s.options.push('… ' + (el.options.length - 12) + ' more');
    } else if (role === 'textbox' || role === 'searchbox' || role === 'spinbutton' || role === 'combobox') {
      const t = (el.getAttribute('type') || '').toLowerCase();
      const v = el.value !== undefined ? el.value : (el.isContentEditable ? el.innerText : '');
      if (t === 'password') {
        if (v) s.value = '(hidden, ' + v.length + ' characters)';
        s.password = true;
      } else if (v) {
        s.value = clean(v, 80);
      }
    }
    if (role === 'heading') {
      const m = /^H([1-6])$/.exec(el.tagName);
      const lv = m ? +m[1] : parseInt(el.getAttribute('aria-level') || '2', 10);
      s.level = Math.min(6, Math.max(1, Number.isFinite(lv) ? lv : 2));
    }
    if (el.tagName === 'A') {
      const link = linkOf(el);
      if (link.href) s.href = clean(link.href, 120);
      if (link.scripted) s.scripted = true;
    }
    if (el === activeIn(el.ownerDocument)) s.focused = true;
    return s;
  };

  // Where a link goes, if it goes anywhere: an http(s) address other than this page's own
  // fragment. `scripted` when something besides the address decides what the click does —
  // Rails' data-method="delete", Turbo's data-turbo-method, an onclick.
  const linkOf = (el) => {
    const out = { href: '', scripted: false };
    const raw = (el.getAttribute('href') || '').trim();
    let proto = '';
    try { proto = (el.protocol || '').toLowerCase(); } catch (e) { proto = ''; }
    const samePage = raw.startsWith('#') || raw === '';
    if ((proto === 'http:' || proto === 'https:') && !samePage) out.href = el.href;
    if (el.hasAttribute('data-method') || el.hasAttribute('data-turbo-method') ||
        el.hasAttribute('onclick') || el.hasAttribute('data-remote')) out.scripted = true;
    return out;
  };

  // The deepest focused element: through shadow roots and same-origin frames.
  const deepActive = (doc) => {
    let a = activeIn(doc || document);
    for (let guard = 0; a && guard < 20; guard++) {
      if (a.shadowRoot && a.shadowRoot.activeElement) { a = a.shadowRoot.activeElement; continue; }
      if (a.tagName === 'IFRAME' || a.tagName === 'FRAME') {
        let inner = null;
        try { inner = a.contentDocument; } catch (e) { inner = null; }
        const next = inner && activeIn(inner);
        if (next && next !== inner.body) { a = next; continue; }
      }
      break;
    }
    return a;
  };

  // The control a press at this element would really press: the element itself when it is one,
  // else the nearest control that holds it.
  const controlOf = (el, win) => {
    for (let up = el, n = 0; up && n < 12; up = up.parentElement || (up.getRootNode && up.getRootNode().host), n++) {
      if (up.nodeType !== 1) continue;
      const role = roleOf(up);
      if (INTERACTIVE_ROLES.has(role)) return up;
      if (!role && HANDMADE_TAGS.has(up.tagName) && handmade(up, up.ownerDocument.defaultView || win)) return up;
    }
    return null;
  };

  // What is around a control, for a judge deciding what pressing it does: the heading it sits
  // under and the text of the form, dialog or section that holds it.
  const HEADINGS = 'h1,h2,h3,h4,[role="heading"]';
  const contextOf = (el) => {
    let heading = '';
    for (let up = el, n = 0; up && !heading && n < 12; up = up.parentElement, n++) {
      for (let sib = up.previousElementSibling, m = 0; sib && m < 20; sib = sib.previousElementSibling, m++) {
        const h = sib.matches(HEADINGS) ? sib : Array.from(sib.querySelectorAll(HEADINGS)).pop();
        if (h) { heading = textOf(h); break; }
      }
    }
    const box = el.closest('form,[role="dialog"],dialog,fieldset,section,article,li') || el.parentElement;
    return { heading: clean(heading, 160), nearby: clean(box ? (box.innerText || '') : '', 400) };
  };

  const describeControl = (c) => {
    const role = roleOf(c) || 'clickable';
    const d = { ref: refOf(c), role, name: nameOf(c) };
    const ctx = contextOf(c);
    d.heading = ctx.heading;
    d.nearby = ctx.nearby;
    if (c.tagName === 'A') {
      const link = linkOf(c);
      if (link.href) d.href = link.href;
      if (link.scripted) d.scripted = true;
    }
    return d;
  };

  const boxOf = (el, frame) => {
    const r = el.getBoundingClientRect();
    return [Math.round(r.left + frame.ox), Math.round(r.top + frame.oy),
            Math.round(r.width), Math.round(r.height)];
  };

  const inView = (b) => b[0] + b[2] > 0 && b[1] + b[3] > 0 &&
    b[0] < window.innerWidth && b[1] < window.innerHeight;

  // The dialog in front, if a modal one is open: what can be done is inside it.
  const openModal = () => {
    for (const [el, frame] of walk(document.documentElement, TOP)) {
      if (frame.depth) continue;
      const role = roleOf(el);
      if (role !== 'dialog' && role !== 'alertdialog') continue;
      const modal = (el.tagName === 'DIALOG' && el.matches(':modal')) || el.getAttribute('aria-modal') === 'true';
      if (modal && !hidden(el, frame.win)) return el;
    }
    return null;
  };

  const composedContains = (outer, node) => {
    while (node) {
      if (node === outer) return true;
      node = node.parentNode || node.host || (node.defaultView && node.defaultView.frameElement) ||
        (node.nodeType === 9 && node.defaultView ? node.defaultView.frameElement : null);
    }
    return false;
  };

  function entry(el, frame, role) {
    const e = { ref: refOf(el), role: role || 'clickable', name: nameOf(el), box: boxOf(el, frame) };
    const s = stateOf(el, role);
    for (const k in s) e[k] = s[k];
    if (frame.depth) e.frame = frame.depth;
    return e;
  }

  function snapshot(all) {
    blindFrames = [];
    const modal = openModal();
    const items = [];
    let above = 0, below = 0, total = 0;
    for (const [el, frame] of walk(document.documentElement, TOP)) {
      let role = roleOf(el);
      let interactive = INTERACTIVE_ROLES.has(role);
      const context = CONTEXT_ROLES.has(role);
      if (!interactive && !context) {
        if (role || !HANDMADE_TAGS.has(el.tagName)) continue;
        if (!inView(boxOf(el, frame)) || !handmade(el, frame.win)) continue;
        interactive = true;
      }
      if (hidden(el, frame.win)) continue;
      if (modal && !composedContains(modal, el) && el !== modal) continue;
      const e = entry(el, frame, role);
      if (context && !e.name) continue;
      if (context) e.context = true;
      total++;
      if (!all && !inView(e.box)) {
        if (e.box[1] + e.box[3] <= 0) above++; else below++;
        continue;
      }
      if (items.length < (all ? 400 : 150)) items.push(e);
    }
    const doc = document.scrollingElement || document.documentElement;
    return {
      url: location.href,
      title: titleOf(document),
      viewport: [window.innerWidth, window.innerHeight],
      scroll: [Math.round(window.scrollX), Math.round(window.scrollY)],
      page: [doc.scrollWidth, doc.scrollHeight],
      modal: modal ? { ref: refOf(modal), name: nameOf(modal) } : null,
      elements: items,
      total: total,
      above: above,
      below: below,
      clipped: items.length < total - above - below,
      blind_frames: blindFrames.map((f) => ({ ref: refOf(f), src: clean(f.src || '', 100),
        box: boxOf(f, TOP) })),
      loading: document.readyState !== 'complete',
    };
  }

  function find(query, limit) {
    const q = (query || '').toLowerCase();
    const out = [];
    for (const [el, frame] of walk(document.documentElement, TOP)) {
      let role = roleOf(el);
      let interactive = INTERACTIVE_ROLES.has(role);
      if (!interactive && !CONTEXT_ROLES.has(role)) {
        if (role || !HANDMADE_TAGS.has(el.tagName)) continue;
        if (!inView(boxOf(el, frame)) || !handmade(el, frame.win)) continue;
      }
      if (hidden(el, frame.win)) continue;
      const e = entry(el, frame, role);
      const hay = (e.name + ' ' + (e.value || '') + ' ' + (e.href || '')).toLowerCase();
      if (!hay.includes(q)) continue;
      e.in_view = inView(e.box);
      out.push(e);
      if (out.length >= (limit || 40)) break;
    }
    return out;
  }

  function text(limit) {
    const parts = [];
    for (const [el, frame] of walk(document.documentElement, TOP)) {
      if (el.shadowRoot && el.shadowRoot.textContent.trim()) {
        const t = Array.from(el.shadowRoot.children).map((c) => c.innerText || '').join('\n').trim();
        if (t) parts.push(t);
      }
      if (frame.depth && el === el.ownerDocument.body) parts.push('[frame] ' + (el.innerText || '').trim());
    }
    let t = (document.body ? document.body.innerText : '') + (parts.length ? '\n\n' + parts.join('\n\n') : '');
    const size = t.length;
    if (limit && t.length > limit) t = t.slice(0, limit);
    return { text: t, size: size, clipped: size > t.length };
  }

  // Where the element is and where its window is, for the moves that need it.
  const frameOf = (el) => {
    let ox = 0, oy = 0;
    let win = el.ownerDocument.defaultView;
    while (win && win !== window && win.frameElement) {
      const f = win.frameElement;
      const r = f.getBoundingClientRect();
      ox += r.left + f.clientLeft;
      oy += r.top + f.clientTop;
      win = f.ownerDocument.defaultView;
    }
    return { ox, oy };
  };

  // What is under a point of the top viewport, through frames and shadow roots.
  const deepHit = (x, y) => {
    let doc = document, lx = x, ly = y, hit = doc.elementFromPoint(lx, ly);
    for (let guard = 0; hit && guard < 20; guard++) {
      if ((hit.tagName === 'IFRAME' || hit.tagName === 'FRAME')) {
        let inner = null;
        try { inner = hit.contentDocument; } catch (e) { inner = null; }
        if (!inner) break;
        const r = hit.getBoundingClientRect();
        lx -= r.left + hit.clientLeft; ly -= r.top + hit.clientTop;
        doc = inner;
        const h = doc.elementFromPoint(lx, ly);
        if (!h || h === hit) break;
        hit = h;
        continue;
      }
      if (hit.shadowRoot) {
        const h = hit.shadowRoot.elementFromPoint(lx, ly);
        if (!h || h === hit) break;
        hit = h;
        continue;
      }
      break;
    }
    return hit;
  };

  function target(ref) {
    const el = byRef(ref);
    if (!el) return { gone: true };
    el.scrollIntoView({ block: 'center', inline: 'center', behavior: 'instant' });
    const f = frameOf(el);
    const r = el.getBoundingClientRect();
    const rects = el.getClientRects();
    // The middle of the first line box for a link that wraps, where a person would press.
    const first = rects.length ? rects[0] : r;
    const x = Math.round(first.left + first.width / 2 + f.ox);
    const y = Math.round(first.top + first.height / 2 + f.oy);
    const role = roleOf(el) || 'clickable';
    const e = { ref, role, name: nameOf(el), x, y, box: [Math.round(r.left + f.ox), Math.round(r.top + f.oy), Math.round(r.width), Math.round(r.height)] };
    if (el.tagName === 'IFRAME' || el.tagName === 'FRAME') e.frame_element = true;
    const ctx = contextOf(el);
    e.heading = ctx.heading;
    e.nearby = ctx.nearby;
    if (CONTEXT_ROLES.has(role)) e.not_a_control = true;
    if (el.tagName === 'A') {
      const link = linkOf(el);
      if (link.href) e.href = link.href;
      if (link.scripted) e.scripted = true;
    }
    if (r.width < 1 || r.height < 1) { e.invisible = true; return e; }
    if (el.disabled || el.getAttribute('aria-disabled') === 'true') e.disabled = true;
    const hit = deepHit(x, y);
    if (hit && !composedContains(el, hit) && !composedContains(hit, el)) {
      // Something else is on top: say what, so the reader can deal with it rather than press it.
      let cover = hit;
      for (let up = hit; up; up = up.parentElement) {
        const role = roleOf(up);
        if (role === 'dialog' || role === 'alertdialog' || up.getAttribute('aria-modal') === 'true') { cover = up; break; }
      }
      e.covered_by = { ref: refOf(cover), role: roleOf(cover) || cover.tagName.toLowerCase(), name: nameOf(cover) };
    } else if (hit) {
      // A press at the middle of a container lands on whatever control is there: that is the
      // one to judge, not the container's name.
      const landing = controlOf(hit, window);
      if (landing && landing !== el) e.lands_on = describeControl(landing);
    }
    const form = el.form || el.closest('form');
    if (form) e.in_form = true;
    // A link that goes somewhere navigates; one with no address or a script is a button.
    const href = el.tagName === 'A' ? (el.getAttribute('href') || '') : '';
    if (href && !href.startsWith('javascript:') && !href.startsWith('#')) e.href = el.href;
    return e;
  }

  function focus(ref, clearFirst) {
    const el = byRef(ref);
    if (!el) return { gone: true };
    el.scrollIntoView({ block: 'center', behavior: 'instant' });
    el.focus();
    if (clearFirst) {
      if (el.select && (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA')) el.select();
      else if (el.isContentEditable) {
        const range = el.ownerDocument.createRange();
        range.selectNodeContents(el);
        const sel = el.ownerDocument.defaultView.getSelection();
        sel.removeAllRanges(); sel.addRange(range);
      }
    }
    const active = deepActive(el.ownerDocument);
    return { focused: active === el || composedContains(el, active), role: roleOf(el), name: nameOf(el),
             password: (el.getAttribute('type') || '').toLowerCase() === 'password' };
  }

  // What Enter or Space may press from here: the focused control itself, every button of its form
  // (form.elements has the ones tied to it from outside with form=), and — for a field with no
  // form, like a chat composer — the buttons in the few containers around it.
  function pressables(ref) {
    const el = (ref && byRef(ref)) || deepActive(document);
    if (!el) return { focused: null, around: [] };
    const out = { focused: describeControl(el), around: [] };
    const seen = new Set();
    const add = (c) => { if (c && !seen.has(c) && c !== el) { seen.add(c); out.around.push(describeControl(c)); } };
    const form = el.form || (el.closest && el.closest('form'));
    if (form) {
      for (const c of Array.from(form.elements || [])) {
        const t = (c.getAttribute('type') || '').toLowerCase();
        if (c.tagName === 'BUTTON' || ['submit', 'image', 'button'].includes(t)) add(c);
      }
      for (const c of Array.from(form.querySelectorAll('[role="button"]'))) add(c);
    } else {
      let up = el.parentElement;
      for (let n = 0; up && n < 4; up = up.parentElement, n++) {
        for (const c of Array.from(up.querySelectorAll('button, [role="button"], input[type="submit"]'))) add(c);
        if (out.around.length) break;
      }
    }
    return out;
  }

  function pick(ref, option, peek) {
    const el = byRef(ref);
    if (!el) return { gone: true };
    if (el.tagName !== 'SELECT') return { not_select: true, role: roleOf(el) };
    const want = (option || '').toLowerCase().trim();
    const opts = Array.from(el.options);
    const o = opts.find((o) => o.text.trim().toLowerCase() === want) ||
      opts.find((o) => o.value.toLowerCase() === want) ||
      opts.find((o) => o.text.toLowerCase().includes(want));
    if (!o) return { no_option: true, options: opts.slice(0, 20).map((o) => clean(o.text, 40)) };
    if (peek) return { would_choose: clean(o.text, 60), list: nameOf(el) };
    el.value = o.value;
    el.dispatchEvent(new Event('input', { bubbles: true }));
    el.dispatchEvent(new Event('change', { bubbles: true }));
    return { chosen: clean(o.text, 60) };
  }

  function scrollBy(dy, ref) {
    if (ref) {
      const el = byRef(ref);
      if (!el) return { gone: true };
      el.scrollIntoView({ block: 'center', behavior: 'instant' });
    } else {
      window.scrollBy(0, dy);
    }
    const doc = document.scrollingElement || document.documentElement;
    return { scroll: [Math.round(window.scrollX), Math.round(window.scrollY)],
             page: [doc.scrollWidth, doc.scrollHeight], viewport: [window.innerWidth, window.innerHeight] };
  }

  // ── what changed ──
  let observer = null, appeared = [], mutations = 0;
  function watch() {
    if (observer) observer.disconnect();
    appeared = []; mutations = 0;
    observer = new MutationObserver((list) => {
      for (const m of list) {
        mutations++;
        if (appeared.length >= 12) continue;
        // Text set with textContent arrives as a text node, and edited text as characterData:
        // either way the element that now says something is the text's parent.
        const nodes = m.type === 'characterData' ? [m.target.parentElement]
          : Array.from(m.addedNodes).map((n) => (n.nodeType === 3 ? n.parentElement : n));
        for (const n of nodes) {
          if (!n || n.nodeType !== 1) continue;
          const t = clean(n.innerText || '', 140);
          if (t.length >= 2 && !appeared.includes(t)) appeared.push(t);
        }
      }
    });
    observer.observe(document, { subtree: true, childList: true, characterData: true });
    return true;
  }
  function changes() {
    if (observer) { observer.takeRecords(); observer.disconnect(); observer = null; }
    // Only what is still there and can be seen: a spinner that came and went is not news.
    const still = appeared.filter((t) => (document.body && document.body.innerText.includes(t.slice(0, 40))));
    return { mutations, appeared: still.slice(0, 8) };
  }

  const where = () => ({ url: location.href, title: titleOf(document) });

  // The page's videos and sounds: playing or not, where, and whether the player says an advert
  // is showing (YouTube's does, in its class names) — a pre-roll transcribes as cleanly as the
  // video and reads like the page's own content.
  function media() {
    const out = [];
    for (const [el, frame] of walk(document.documentElement, TOP)) {
      if (el.tagName !== 'VIDEO' && el.tagName !== 'AUDIO') continue;
      const player = el.closest('.html5-video-player');
      const ad = !!(player && (player.classList.contains('ad-showing') || player.classList.contains('ad-interrupting')));
      out.push({ ref: refOf(el), kind: el.tagName.toLowerCase(), playing: !el.paused && !el.ended,
                 at: Math.round(el.currentTime || 0), length: Math.round(el.duration || 0) || null,
                 muted: el.muted, ad: ad || undefined, box: boxOf(el, frame) });
    }
    return out;
  }

  globalThis.__yb = { __yantrik: true, snapshot, find, text, target, focus, pressables, pick,
                      scrollBy, watch, changes, where, media };
  return 'installed';
})()
