// What changed on the page between an action and its settling, and where a point lands: the
// watching part of the reader (page.js), evaluated in the same isolated world right after it.
// It builds on the helpers page.js leaves on globalThis.__yb (clean, deepHit, controlOf,
// describeControl) and adds, beside them:
//   watch()/changes()  what appeared on the page between an action and its settling, and
//                      whether a right click met a menu of the page's own.
//   at(x, y)           the control a press at this point would land on: where a drag would drop.
(() => {
  const yb = globalThis.__yb;
  if (!yb || yb.__yantrik !== true) return 'no reader';
  if (yb.at) return 'ready';
  const { clean, deepHit, controlOf, describeControl } = yb;

  // The control a press at this point of the top viewport would land on, described for a judge:
  // where a drag would drop. A frame from another site is no control, but a press into it is
  // one that cannot be read: it is named as a frame.
  function at(x, y) {
    const hit = deepHit(x, y);
    if (hit && (hit.tagName === 'IFRAME' || hit.tagName === 'FRAME')) return { frame_element: true };
    const landing = hit && controlOf(hit, window);
    return landing ? describeControl(landing) : null;
  }

  let observer = null, appeared = [], mutations = 0;
  // A right click: whether the page answered with a menu of its own (it called preventDefault
  // on contextmenu) — and the browser's own menu is never let open, since nothing on this
  // surface could read or close it, and in a window it would eat the next click.
  let menuSeen = null, menuCapture = null, menuBubble = null;
  const unwatchMenu = () => {
    if (menuCapture) window.removeEventListener('contextmenu', menuCapture, true);
    if (menuBubble) window.removeEventListener('contextmenu', menuBubble, false);
    menuCapture = menuBubble = null;
  };
  function watch() {
    if (observer) observer.disconnect();
    appeared = []; mutations = 0;
    unwatchMenu();
    menuSeen = null;
    menuCapture = () => { menuSeen = 'unknown'; };
    menuBubble = (e) => { menuSeen = e.defaultPrevented ? 'page' : 'none'; e.preventDefault(); };
    window.addEventListener('contextmenu', menuCapture, true);
    window.addEventListener('contextmenu', menuBubble, false);
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
    unwatchMenu();
    // Only what is still there and can be seen: a spinner that came and went is not news.
    const still = appeared.filter((t) => (document.body && document.body.innerText.includes(t.slice(0, 40))));
    const out = { mutations, appeared: still.slice(0, 8) };
    if (menuSeen) out.context_menu = menuSeen;
    return out;
  }

  Object.assign(yb, { watch, changes, at });
  return 'installed';
})()
