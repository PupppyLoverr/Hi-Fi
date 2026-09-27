//! JS injected into webviews for the browser-use surface (snapshot/click/type).

/// Browser context menus are routed through the existing page IPC bridge.
pub const CONTEXT_MENU_JS: &str = r#"document.addEventListener('contextmenu',e=>{e.preventDefault();window.ipc&&window.ipc.postMessage(JSON.stringify({type:'ctx',x:e.clientX,y:e.clientY,href:(e.target.closest('a')||{}).href||'',src:e.target.tagName==='IMG'?e.target.src:'',sel:String(getSelection())}))},true);document.addEventListener('mousedown',e=>{if(e.button===0&&window.ipc)window.ipc.postMessage(JSON.stringify({type:'page-click'}))},true)"#;

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
    el.setAttribute('data-hifi', out.length);
    const sel = '[data-hifi="' + out.length + '"]';
    out.push({ i: out.length, role, sel, label: String(label).slice(0, 80).trim(), x: Math.round(r.x), y: Math.round(r.y) });
  }
  return JSON.stringify({ url: location.href, title: document.title, elements: out });
})()"#;

/// Page text for agents: title, url and the visible innerText, capped so a
/// long page doesn't blow up the model context.
pub const READ_JS: &str = r#"(() => {
  const text = (document.body?.innerText || '').replace(/\n{3,}/g, '\n\n').slice(0, 20000);
  return JSON.stringify({ url: location.href, title: document.title, text });
})()"#;

/// Human-style click: scroll into view, then dispatch the pointer sequence
/// a real click produces (so pages listening for pointer/mouse events react).
/// A bare snapshot index ("3") addresses the element tagged by the last
/// snapshot; anything else is a CSS selector.
fn target_selector(target: &str) -> String {
    let t = target.trim();
    if !t.is_empty() && t.chars().all(|c| c.is_ascii_digit()) {
        format!("[data-hifi=\"{t}\"]")
    } else {
        t.to_string()
    }
}

pub fn click_js(selector: &str) -> String {
    let selector = target_selector(selector);
    format!(
        r#"(() => {{
  const el = document.querySelector({sel});
  if (!el) return JSON.stringify({{ ok: false, error: 'no element matches selector' }});
  el.scrollIntoView({{ block: 'center', inline: 'center' }});
  const r = el.getBoundingClientRect();
  const o = {{ bubbles: true, cancelable: true, clientX: r.x + r.width / 2, clientY: r.y + r.height / 2, button: 0 }};
  for (const t of ['pointerdown', 'mousedown', 'pointerup', 'mouseup']) el.dispatchEvent(new (t.startsWith('pointer') ? PointerEvent : MouseEvent)(t, o));
  el.focus && el.focus();
  el.click();
  return JSON.stringify({{ ok: true, label: (el.innerText || el.value || el.getAttribute('aria-label') || '').slice(0, 80) }});
}})()"#,
        sel = serde_json::to_string(&selector).unwrap_or_default()
    )
}

/// Human-style typing: focus, replace the value, fire input/change; `submit`
/// presses Enter afterwards (form submit or keydown handlers).
pub fn type_js(selector: &str, text: &str, submit: bool) -> String {
    let selector = target_selector(selector);
    format!(
        r#"(() => {{
  const el = document.querySelector({sel});
  if (!el) return JSON.stringify({{ ok: false, error: 'no element matches selector' }});
  el.scrollIntoView({{ block: 'center' }});
  el.focus();
  if (el.isContentEditable) el.textContent = {text}; else {{
    const proto = Object.getPrototypeOf(el);
    const d = Object.getOwnPropertyDescriptor(proto, 'value');
    d && d.set ? d.set.call(el, {text}) : (el.value = {text});
  }}
  el.dispatchEvent(new Event('input', {{ bubbles: true }}));
  el.dispatchEvent(new Event('change', {{ bubbles: true }}));
  if ({submit}) {{
    const k = {{ key: 'Enter', code: 'Enter', keyCode: 13, which: 13, bubbles: true, cancelable: true }};
    const stop = !el.dispatchEvent(new KeyboardEvent('keydown', k));
    el.dispatchEvent(new KeyboardEvent('keypress', k));
    el.dispatchEvent(new KeyboardEvent('keyup', k));
    if (!stop && el.form) el.form.requestSubmit ? el.form.requestSubmit() : el.form.submit();
  }}
  return JSON.stringify({{ ok: true }});
}})()"#,
        sel = serde_json::to_string(&selector).unwrap_or_default(),
        text = serde_json::to_string(text).unwrap_or_default(),
        submit = submit
    )
}

pub fn scroll_js(dy: f64) -> String {
    format!(
        "(()=>{{window.scrollBy({{top:{dy},behavior:'instant'}});return JSON.stringify({{ok:true,y:Math.round(window.scrollY),max:Math.round(document.documentElement.scrollHeight-window.innerHeight)}})}})()"
    )
}
