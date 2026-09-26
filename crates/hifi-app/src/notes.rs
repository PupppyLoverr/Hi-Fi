//! Pages — a Notion-style block editor rendered inside a WKWebView pane
//! (`hifi://notes`). Editing is native contenteditable: a page title, a `/`
//! block menu, markdown shortcuts (`#`, `-`, `1.`, `[]`, `>`, `!`, ```` ``` ````,
//! `---`) and to-do checkboxes. The page autosaves `{title, html}` back
//! through wry's `window.ipc.postMessage` channel.

/// Payload the editor posts on every autosave.
#[derive(serde::Deserialize)]
pub struct PageSave {
    pub title: String,
    pub html: String,
}

/// The full document for a page. `body` is previously-saved innerHTML and
/// `title` the page title; `dark` matches the app theme so nothing flashes.
pub fn editor_html(body: &str, title: &str, dark: bool) -> String {
    let (bg, text, muted, faint, accent, border, wash, menu) = if dark {
        (
            "#0a0a0a",
            "#e8e8ea",
            "#a9a9ae",
            "#85858a",
            "#8b7cf6",
            "rgba(255,255,255,0.08)",
            "rgba(235,235,235,0.08)",
            "#161616",
        )
    } else {
        (
            "#ffffff",
            "#303035",
            "#62626a",
            "#797981",
            "#5b43e8",
            "rgba(0,0,0,0.08)",
            "rgba(26,26,26,0.05)",
            "#ffffff",
        )
    };
    let title = if title == "Notes" { "" } else { title };
    format!(
        r#"<!doctype html><html><head><meta charset="utf-8"><style>
* {{ box-sizing: border-box; }}
html, body {{ margin: 0; min-height: 100%; }}
body {{
  background: {bg}; color: {text};
  font: 15.5px/1.65 -apple-system, BlinkMacSystemFont, "Inter", sans-serif;
  padding: 56px 48px 50vh; overflow-wrap: break-word;
  -webkit-font-smoothing: antialiased;
}}
main {{ max-width: 720px; margin: 0 auto; }}
#title {{
  outline: none; font-size: 2.35em; font-weight: 700; line-height: 1.2;
  letter-spacing: -0.02em; margin: 0 0 0.6em; caret-color: {accent};
}}
#title:empty::before {{ content: "Untitled"; color: {faint}; opacity: .6; }}
#doc {{ outline: none; caret-color: {accent}; }}
#doc > * {{ position: relative; }}
.ph::after {{
  content: "Type '/' for blocks"; color: {faint}; opacity: .55;
  position: absolute; left: 0; top: 0; pointer-events: none;
}}
h1 {{ font-size: 1.75em; font-weight: 700; margin: 1em 0 0.25em; letter-spacing: -0.015em; }}
h2 {{ font-size: 1.4em; font-weight: 650; margin: 0.9em 0 0.2em; }}
h3 {{ font-size: 1.15em; font-weight: 600; margin: 0.8em 0 0.15em; }}
p, div {{ margin: 0.2em 0; }}
blockquote {{ margin: 0.4em 0; padding: 0.1em 0 0.1em 1em; border-left: 3px solid {text}; }}
ul, ol {{ margin: 0.2em 0; padding-left: 1.6em; }}
li {{ margin: 0.1em 0; }}
ul.todo {{ list-style: none; padding-left: 0.2em; }}
ul.todo li {{ display: flex; gap: 0.55em; align-items: baseline; }}
ul.todo li::before {{
  content: ""; flex: none; width: 14px; height: 14px; border-radius: 4px;
  border: 1.5px solid {faint}; transform: translateY(2px); cursor: pointer;
}}
ul.todo li.done {{ color: {faint}; text-decoration: line-through; }}
ul.todo li.done::before {{ background: {accent}; border-color: {accent}; }}
.callout {{
  display: flex; gap: 0.6em; padding: 0.8em 1em; border-radius: 10px;
  background: {wash}; margin: 0.5em 0;
}}
.callout::before {{ content: "💡"; }}
pre {{
  font-family: "Geist Mono", ui-monospace, monospace; font-size: 0.86em; line-height: 1.55;
  background: {wash}; border: 1px solid {border}; border-radius: 10px;
  padding: 0.9em 1em; margin: 0.5em 0; white-space: pre-wrap;
}}
code {{ font-family: "Geist Mono", ui-monospace, monospace; font-size: 0.86em;
       background: {wash}; border-radius: 4px; padding: 0.1em 0.35em; color: #eb5757; }}
hr {{ border: none; border-top: 1px solid {border}; margin: 1.2em 0; }}
a {{ color: {accent}; }}
::selection {{ background: {accent}44; }}
#menu {{
  position: absolute; display: none; width: 240px; padding: 4px;
  background: {menu}; border: 1px solid {border}; border-radius: 10px;
  box-shadow: 0 12px 32px rgba(0,0,0,.28); z-index: 10; font-size: 13px;
}}
#menu .h {{ color: {faint}; font-size: 11px; padding: 6px 8px 4px; }}
#menu .i {{ display: flex; gap: 10px; align-items: center; padding: 6px 8px; border-radius: 6px; cursor: pointer; }}
#menu .i b {{ width: 22px; height: 22px; border: 1px solid {border}; border-radius: 5px;
  display: grid; place-items: center; font-weight: 600; font-size: 11px; color: {muted}; }}
#menu .i span {{ color: {text}; }}
#menu .i small {{ margin-left: auto; color: {faint}; font-family: "Geist Mono", monospace; font-size: 10.5px; }}
#menu .i.on {{ background: {wash}; }}
</style></head><body><main>
<div id="title" contenteditable="true" spellcheck="false"></div>
<div id="doc" contenteditable="true"></div>
</main><div id="menu"></div><script>
const doc = document.getElementById('doc');
const titleEl = document.getElementById('title');
const menu = document.getElementById('menu');
doc.innerHTML = {body_json} || '<div><br></div>';
titleEl.textContent = {title_json};

let saveTimer = null;
function save() {{
  if (window.ipc) window.ipc.postMessage(JSON.stringify({{
    title: titleEl.textContent.trim(), html: doc.innerHTML }}));
}}
function queue() {{ clearTimeout(saveTimer); saveTimer = setTimeout(save, 500); placeholder(); }}
doc.addEventListener('input', queue);
titleEl.addEventListener('input', queue);
window.addEventListener('beforeunload', save);
titleEl.addEventListener('keydown', (e) => {{
  if (e.key === 'Enter') {{ e.preventDefault(); caretTo(doc.firstElementChild || doc, true); }}
}});

function block() {{
  const sel = getSelection(); if (!sel.rangeCount) return null;
  let el = sel.getRangeAt(0).startContainer;
  while (el && el.parentNode !== doc) el = el.parentNode;
  return el && el.parentNode === doc ? el : null;
}}
function leaf() {{
  const sel = getSelection(); if (!sel.rangeCount) return null;
  let el = sel.getRangeAt(0).startContainer;
  if (el.nodeType === 3) el = el.parentElement;
  return el;
}}
function caretTo(el, start) {{
  const r = document.createRange(); r.selectNodeContents(el); r.collapse(!!start);
  const s = getSelection(); s.removeAllRanges(); s.addRange(r); el.focus && doc.focus();
}}
function placeholder() {{
  doc.querySelectorAll('.ph').forEach(n => n.classList.remove('ph'));
  const b = block();
  if (b && b.tagName === 'DIV' && !b.className && b.textContent === '') b.classList.add('ph');
}}
document.addEventListener('selectionchange', placeholder);

const BLOCKS = [
  ['T', 'Text', '', () => make('div')],
  ['H1', 'Heading 1', '#', () => make('h1')],
  ['H2', 'Heading 2', '##', () => make('h2')],
  ['H3', 'Heading 3', '###', () => make('h3')],
  ['☐', 'To-do list', '[]', () => list('ul', 'todo')],
  ['•', 'Bulleted list', '-', () => list('ul')],
  ['1.', 'Numbered list', '1.', () => list('ol')],
  ['❝', 'Quote', '>', () => make('blockquote')],
  ['!', 'Callout', '!', () => make('div', 'callout')],
  ['{{}}', 'Code', '```', () => make('pre')],
  ['—', 'Divider', '---', () => divider()],
];
function make(tag, cls) {{
  const b = block(); const n = document.createElement(tag);
  if (cls) n.className = cls;
  n.innerHTML = (b && b.textContent) ? b.innerHTML : '<br>';
  if (b) b.replaceWith(n); else doc.appendChild(n);
  caretTo(n, false); queue();
}}
function list(tag, cls) {{
  const b = block(); const l = document.createElement(tag);
  if (cls) l.className = cls;
  const li = document.createElement('li');
  li.innerHTML = (b && b.textContent) ? b.innerHTML : '<br>';
  l.appendChild(li);
  if (b) b.replaceWith(l); else doc.appendChild(l);
  caretTo(li, false); queue();
}}
function divider() {{
  const b = block(); const hr = document.createElement('hr');
  const next = document.createElement('div'); next.innerHTML = '<br>';
  if (b) {{ b.replaceWith(hr); }} else doc.appendChild(hr);
  hr.after(next); caretTo(next, true); queue();
}}

// Slash menu.
let menuOn = false, menuSel = 0, menuItems = [], slashNode = null, slashAt = 0;
function renderMenu(q) {{
  menuItems = BLOCKS.filter(b => b[1].toLowerCase().includes(q.toLowerCase()));
  if (!menuItems.length) {{ closeMenu(); return; }}
  menuSel = Math.min(menuSel, menuItems.length - 1);
  menu.innerHTML = '<div class="h">Basic blocks</div>' + menuItems.map((b, i) =>
    `<div class="i${{i === menuSel ? ' on' : ''}}" data-i="${{i}}"><b>${{b[0]}}</b><span>${{b[1]}}</span><small>${{b[2]}}</small></div>`).join('');
}}
function openMenu() {{
  const sel = getSelection(); const r = sel.getRangeAt(0).cloneRange();
  slashNode = r.startContainer; slashAt = r.startOffset;
  const rect = r.getBoundingClientRect();
  menu.style.left = (rect.left + scrollX) + 'px';
  menu.style.top = (rect.bottom + scrollY + 6) + 'px';
  menu.style.display = 'block'; menuOn = true; menuSel = 0; renderMenu('');
}}
function closeMenu() {{ menu.style.display = 'none'; menuOn = false; }}
function query() {{
  if (!slashNode || !slashNode.textContent) return '';
  const r = getSelection().getRangeAt(0);
  return slashNode.textContent.slice(slashAt, r.startOffset);
}}
function pick(i) {{
  const b = menuItems[i]; if (!b) return;
  const q = query();
  if (slashNode && slashNode.nodeType === 3) {{
    const t = slashNode.textContent;
    slashNode.textContent = t.slice(0, slashAt - 1) + t.slice(slashAt + q.length);
  }}
  closeMenu(); b[3]();
}}
menu.addEventListener('mousedown', (e) => {{
  const it = e.target.closest('.i'); if (!it) return;
  e.preventDefault(); pick(+it.dataset.i);
}});
doc.addEventListener('keyup', (e) => {{
  if (!menuOn) return;
  if (['ArrowUp','ArrowDown','Enter','Escape'].includes(e.key)) return;
  const q = query();
  if (q === null || q.includes(' ') || getSelection().getRangeAt(0).startContainer !== slashNode) closeMenu();
  else renderMenu(q);
}});

doc.addEventListener('keydown', (e) => {{
  if (menuOn) {{
    if (e.key === 'ArrowDown') {{ e.preventDefault(); menuSel = (menuSel + 1) % menuItems.length; renderMenu(query()); return; }}
    if (e.key === 'ArrowUp') {{ e.preventDefault(); menuSel = (menuSel + menuItems.length - 1) % menuItems.length; renderMenu(query()); return; }}
    if (e.key === 'Enter') {{ e.preventDefault(); pick(menuSel); return; }}
    if (e.key === 'Escape') {{ e.preventDefault(); closeMenu(); return; }}
  }}
  if (e.key === '/') {{ setTimeout(openMenu, 0); return; }}
  if (e.key === ' ') {{
    const sel = getSelection(); if (!sel.rangeCount) return;
    const r = sel.getRangeAt(0); const node = r.startContainer;
    const head = (node.textContent || '').slice(0, r.startOffset);
    const hit = BLOCKS.find(b => b[2] && b[2] === head);
    const b = block();
    if (hit && b && b.textContent.startsWith(head) && !['UL','OL','PRE'].includes(b.tagName)) {{
      e.preventDefault();
      node.textContent = node.textContent.slice(r.startOffset);
      hit[3]();
    }}
    return;
  }}
  if (e.key === 'Enter' && !e.shiftKey) {{
    const b = block(); const el = leaf(); if (!b) return;
    if (b.tagName === 'PRE') return;
    const li = el && el.closest('li');
    if (li && li.textContent === '') {{
      e.preventDefault();
      const div = document.createElement('div'); div.innerHTML = '<br>';
      const l = li.parentNode; li.remove(); l.after(div);
      if (!l.children.length) l.remove();
      caretTo(div, true); queue(); return;
    }}
    if (li && l_is_todo(li)) {{
      setTimeout(() => {{ const n = leaf(); const nl = n && n.closest('li'); if (nl) nl.classList.remove('done'); }}, 0);
      return;
    }}
    if (['H1','H2','H3','BLOCKQUOTE'].includes(b.tagName) || b.classList.contains('callout')) {{
      e.preventDefault();
      const div = document.createElement('div'); div.innerHTML = '<br>';
      b.after(div); caretTo(div, true); queue();
    }}
  }}
  if (e.key === 'Backspace') {{
    const b = block(); const sel = getSelection();
    if (b && sel.isCollapsed && sel.getRangeAt(0).startOffset === 0 && b.textContent === ''
        && b.tagName !== 'DIV') {{
      e.preventDefault(); const div = document.createElement('div'); div.innerHTML = '<br>';
      b.replaceWith(div); caretTo(div, true); queue();
    }}
  }}
}});
function l_is_todo(li) {{ return li.parentNode.classList.contains('todo'); }}
doc.addEventListener('mousedown', (e) => {{
  const li = e.target.closest && e.target.closest('ul.todo li');
  if (li && e.offsetX < 18) {{ e.preventDefault(); li.classList.toggle('done'); queue(); }}
}});
doc.addEventListener('paste', (e) => {{
  e.preventDefault();
  const t = (e.clipboardData || window.clipboardData).getData('text/plain');
  document.execCommand('insertText', false, t);
}});
(titleEl.textContent ? doc : titleEl).focus();
</script></body></html>"#,
        bg = bg,
        text = text,
        muted = muted,
        faint = faint,
        accent = accent,
        border = border,
        wash = wash,
        menu = menu,
        body_json = serde_json::to_string(body).unwrap_or_else(|_| "\"\"".into()),
        title_json = serde_json::to_string(title).unwrap_or_else(|_| "\"\"".into()),
    )
}
