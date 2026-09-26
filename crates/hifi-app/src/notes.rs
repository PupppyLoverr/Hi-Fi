//! Notes surface — a Notion-style editor document rendered inside a
//! WKWebView pane (`hifi://notes`). Editing is native contenteditable;
//! markdown-ish shortcuts (`# `, `- `, `> `, `---`) reshape blocks and the
//! body autosaves back through wry's `window.ipc.postMessage` channel —
//! persisted one file per tab under the state dir.

/// The full document for a notes tab. `body` is previously-saved innerHTML.
/// `dark` matches the app theme so the page doesn't flash white.
pub fn editor_html(body: &str, dark: bool) -> String {
    let (bg, text, muted, accent, border) = if dark {
        ("#0a0a0a", "#e8e8ea", "#85858a", "#8b7cf6", "rgba(255,255,255,0.08)")
    } else {
        ("#ffffff", "#303035", "#797981", "#5b43e8", "rgba(0,0,0,0.08)")
    };
    format!(
        r#"<!doctype html><html><head><meta charset="utf-8"><style>
* {{ box-sizing: border-box; }}
html, body {{ margin: 0; height: 100%; }}
body {{
  background: {bg}; color: {text};
  font: 16px/1.65 -apple-system, "Geist", sans-serif;
  padding: 28px 32px 60vh; overflow-wrap: break-word;
}}
#doc {{ outline: none; max-width: 720px; margin: 0 auto; caret-color: {accent}; }}
#doc:empty::before, #doc > :first-child[data-empty]::before {{
  content: "Start writing — '#' for a heading, '-' for a list, '>' for a quote";
  color: {muted}; pointer-events: none; position: absolute;
}}
h1 {{ font-size: 1.9em; font-weight: 700; margin: 0.8em 0 0.3em; }}
h2 {{ font-size: 1.45em; font-weight: 650; margin: 0.9em 0 0.25em; }}
h3 {{ font-size: 1.15em; font-weight: 600; margin: 0.9em 0 0.2em; }}
p, div {{ margin: 0.25em 0; }}
blockquote {{
  margin: 0.4em 0; padding: 0.15em 0 0.15em 1em;
  border-left: 3px solid {accent}; color: {muted};
}}
ul {{ margin: 0.25em 0; padding-left: 1.5em; }}
hr {{ border: none; border-top: 1px solid {border}; margin: 1.4em 0; }}
code {{ font-family: "Geist Mono", monospace; font-size: 0.86em;
       background: {border}; border-radius: 4px; padding: 0.1em 0.35em; }}
::selection {{ background: {accent}55; }}
</style></head><body><div id="doc" contenteditable="true"></div><script>
const doc = document.getElementById('doc');
doc.innerHTML = {body_json} || '<div><br></div>';

let saveTimer = null;
function save() {{
  if (window.ipc) window.ipc.postMessage(doc.innerHTML);
}}
doc.addEventListener('input', () => {{
  clearTimeout(saveTimer); saveTimer = setTimeout(save, 600);
}});
window.addEventListener('beforeunload', save);

// Notion-ish block reshaping on space: '#', '##', '###', '-', '>', '```', '---'
doc.addEventListener('keydown', (e) => {{
  if (e.key === ' ') {{
    const sel = getSelection();
    if (!sel.rangeCount) return;
    const r = sel.getRangeAt(0);
    const node = r.startContainer;
    const text = node.textContent || '';
    const head = text.slice(0, r.startOffset);
    const map = {{'#':'h1','##':'h2','###':'h3','>':'blockquote'}};
    if (map[head]) {{
      e.preventDefault();
      node.textContent = text.slice(r.startOffset);
      document.execCommand('formatBlock', false, map[head]);
    }} else if (head === '-') {{
      e.preventDefault();
      node.textContent = text.slice(r.startOffset);
      document.execCommand('insertUnorderedList', false, null);
    }} else if (head === '---') {{
      e.preventDefault();
      node.textContent = text.slice(r.startOffset);
      document.execCommand('insertHorizontalRule', false, null);
    }}
  }}
  if (e.key === 'Enter' && !e.shiftKey) {{
    // Leave headings/quotes on Enter: next block is a normal div.
    const sel = getSelection();
    if (sel.rangeCount) {{
      let el = sel.getRangeAt(0).startContainer;
      if (el.nodeType === 3) el = el.parentElement;
      const tag = el && el.tagName && el.tagName.toLowerCase();
      if (['h1','h2','h3','blockquote'].includes(tag)) {{
        e.preventDefault();
        const div = document.createElement('div');
        div.innerHTML = '<br>';
        el.after(div);
        const r = document.createRange();
        r.setStart(div, 0); r.collapse(true);
        sel.removeAllRanges(); sel.addRange(r);
      }}
    }}
  }}
}});
// Paste as plain text — keeps docs clean of foreign markup.
doc.addEventListener('paste', (e) => {{
  e.preventDefault();
  const t = (e.clipboardData || window.clipboardData).getData('text/plain');
  document.execCommand('insertText', false, t);
}});
doc.focus();
</script></body></html>"#,
        bg = bg,
        text = text,
        muted = muted,
        accent = accent,
        border = border,
        body_json = serde_json::to_string(body).unwrap_or_else(|_| "\"\"".into()),
    )
}
