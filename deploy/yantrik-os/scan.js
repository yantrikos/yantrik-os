// The page reader of the companion's own browser tools, which compile it in
// (crates/yantrik-companion-tools/src/browser.rs, `SCAN_ELEMENTS_JS`): every interactive element
// on the page, numbered, with the name a screen reader would give it, leading with what is on
// screen. `yos web` no longer uses it — it is a client of the browser's surface (apps/browser,
// #477), whose reader is apps/browser/yantrik_browser/page.js. It lived
// only inside that Rust file, `yos` looked for a scan.js that was never committed or shipped,
// and so `yos web`, `web_read`, `web_find` and clicking by ref failed on every published build.
//
// `const showAll = false;` is rewritten to `true` by both callers for the whole-page listing —
// keep that line exactly as it is.
(() => {
    const showAll = false;
    const sels = 'a[href], button, input, textarea, select, [role="button"], [role="link"], [role="tab"], [role="menuitem"], [contenteditable="true"], summary, details';

    // The accessible name, in the order the accname algorithm resolves it.
    const nameOf = (el) => {
        const byIds = (ids) => ids.split(/\s+/)
            .map(id => document.getElementById(id))
            .filter(Boolean)
            .map(n => (n.innerText || n.textContent || '').trim())
            .join(' ')
            .trim();

        const labelledby = el.getAttribute('aria-labelledby');
        if (labelledby) { const t = byIds(labelledby); if (t) return t; }

        const aria = el.getAttribute('aria-label');
        if (aria && aria.trim()) return aria.trim();

        if (el.id) {
            const lab = document.querySelector('label[for="' + CSS.escape(el.id) + '"]');
            if (lab) { const t = (lab.innerText || lab.textContent || '').trim(); if (t) return t; }
        }
        const wrapping = el.closest('label');
        if (wrapping) { const t = (wrapping.innerText || wrapping.textContent || '').trim(); if (t) return t; }

        const own = (el.innerText || el.textContent || '').trim();
        if (own) return own;

        return (el.placeholder || el.value || el.alt || el.title || '').trim();
    };

    const els = [];
    document.querySelectorAll(sels).forEach(el => {
        const r = el.getBoundingClientRect();
        if (r.width === 0 && r.height === 0 && el.tagName !== 'INPUT') return;
        if (el.disabled) return;
        if (el.closest('[aria-hidden="true"]') && !el.closest('[aria-modal="true"]')) return;
        els.push(el);
    });
    // Indexed before filtering, so an index means the same thing whatever is scrolled into view.
    window.__yantrik_elements = els;

    const visible = [];
    const lines = [];
    els.forEach((el, i) => {
        const r = el.getBoundingClientRect();
        const onscreen = r.bottom > 0 && r.top < innerHeight && r.right > 0 && r.left < innerWidth;

        const tag = el.tagName.toLowerCase();
        const type = el.type || '';
        const role = el.getAttribute('role') || '';
        let text = nameOf(el).replace(/\s+/g, ' ');
        if (text.length > 60) text = text.substring(0, 57) + '...';
        const name = el.name || el.id || '';
        const href = el.href || '';

        let desc = '[' + (i+1) + '] ' + tag;
        if (type && type !== 'submit') desc += '[' + type + ']';
        if (role) desc += '[' + role + ']';
        if (name) desc += ' name="' + name + '"';
        if (text) desc += ' "' + text + '"';
        if (href && tag === 'a') {
            try { desc += ' \u2192 ' + new URL(href).pathname.substring(0, 60); } catch(e) { desc += ' \u2192 ' + href.substring(0, 60); }
        }
        if (tag === 'input' || tag === 'textarea') {
            const val = el.value || '';
            if (val) desc += ' value="' + val.substring(0, 40) + '"';
        }
        if (onscreen || showAll) visible.push(desc); else lines.push(desc);
    });

    const offscreen = lines.length;
    let out = visible.join('\n');
    if (offscreen > 0) {
        // Said rather than silently dropped: an agent that cannot see something must know it is
        // there, or it will conclude the page does not have it and give up.
        out += '\n\n(' + offscreen + ' more not currently on screen \u2014 scroll, or ask for all)';
    }
    return out;
})()
