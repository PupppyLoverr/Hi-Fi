//! JS injected into webviews for the browser-use surface (snapshot/click/type).

/// Serialized DOM snapshot: interactive elements with indexes + labels.
pub const SNAPSHOT_JS: &str = r#"(() => {
  const els = Array.from(document.querySelectorAll(
    'a,button,input,select,textarea,[role=button],[role=link],[contenteditable=true],label'
  ));
  const out = [];
  for (const el of els.slice(0, 500)) {
    const r = el.getBoundingClientRect();
    if (r.width === 0 || r.height === 0) continue;
    const label = el.getAttribute('aria-label') || el.innerText || el.value || el.name || el.id || el.tagName;
    const role = el.getAttribute('role') || el.tagName.toLowerCase();
    const sel = el.id ? '#' + el.id
      : el.name ? `[name="${el.name}"]`
      : role + ':nth-of-type(' + (Array.from(el.parentElement?.children || []).filter(c => c.tagName === el.tagName).indexOf(el) + 1) + ')';
    out.push({ i: out.length, role, sel, label: String(label).slice(0, 80).trim(), x: Math.round(r.x), y: Math.round(r.y) });
  }
  return JSON.stringify({ url: location.href, title: document.title, elements: out });
})()"#;
