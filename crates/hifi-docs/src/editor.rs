use crate::{Block, DocKind, Document};
use base64::Engine;

pub fn editor_html(doc: &Document, path: &str, pdf_bytes: Option<&[u8]>, dark: bool) -> String {
    let (bg, text, muted, faint, accent, border, wash) = if dark {
        (
            "#0a0a0a",
            "#e8e8ea",
            "#a9a9ae",
            "#85858a",
            "#8b7cf6",
            "rgba(255,255,255,0.08)",
            "rgba(235,235,235,0.08)",
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
        )
    };
    let filename = path.rsplit(['/', '\\']).next().unwrap_or(path);
    if doc.kind == DocKind::Pdf {
        let embed = pdf_bytes.map(|bytes| {
            let data = base64::engine::general_purpose::STANDARD.encode(bytes);
            format!("<embed class=\"pdf\" type=\"application/pdf\" src=\"data:application/pdf;base64,{data}\" onerror=\"document.getElementById('fallback').hidden=false\" />")
        }).unwrap_or_default();
        let fallback = doc
            .blocks
            .iter()
            .map(block_plain)
            .filter(|s| !s.is_empty())
            .map(|s| format!("<p>{}</p>", html(&s)))
            .collect::<String>();
        format!(
            r#"<!doctype html><html><head><meta charset="utf-8"><title>{}</title><style>{}.pdf{{width:100%;height:calc(100vh - 82px);border:0}}.label{{padding:10px 18px;color:{muted};font-size:12px;border-bottom:1px solid {border}}}.fallback{{max-width:760px;margin:32px auto;line-height:1.6}}</style></head><body><header class="label">Read-only PDF · {}</header>{embed}<main id="fallback" class="fallback" hidden>{fallback}</main></body></html>"#,
            html(filename),
            base_css(bg, text, muted, faint, accent, border, wash),
            html(filename)
        )
    } else {
        let data = serde_json::to_string(doc)
            .unwrap_or_else(|_| "null".into())
            .replace('<', "\\u003c")
            .replace('>', "\\u003e")
            .replace('&', "\\u0026");
        let app = match doc.kind {
            DocKind::Docx => docx_app(),
            DocKind::Xlsx => xlsx_app(),
            DocKind::Pptx => pptx_app(),
            DocKind::Pdf => String::new(),
        };
        let keyboard = editor_keyboard_js();
        format!(
            r#"<!doctype html><html><head><meta charset="utf-8"><title>{}</title><style>{}</style></head><body><header><div class="filename">{}</div><div class="tools" id="toolbar"></div></header><main id="app"></main><script>const d={data};{keyboard}{app}</script></body></html>"#,
            html(filename),
            base_css(bg, text, muted, faint, accent, border, wash),
            html(filename)
        )
    }
}

fn editor_keyboard_js() -> &'static str {
    r#"
const editorSelector='#app [contenteditable="true"],#app input,#app textarea';
const editorKeys=new Set(['Backspace','Delete','ArrowLeft','ArrowRight','ArrowUp','ArrowDown','PageUp','PageDown','Home','End','Tab','Enter','Escape','Insert']);
const privateUse=c=>{const n=c.codePointAt(0)||0;return (n>=0xE000&&n<=0xF8FF)||(n>=0xF0000&&n<=0xFFFFD)||(n>=0x100000&&n<=0x10FFFD)};
const invalidText=c=>{const n=c.codePointAt(0)||0;return (n<0x20&&c!=='\t'&&c!=='\n')||privateUse(c)};
const editorField=target=>target instanceof Element?target.closest(editorSelector):null;
document.addEventListener('beforeinput',e=>{if(editorField(e.target)&&typeof e.data==='string'&&Array.from(e.data).some(invalidText))e.preventDefault()},true);
document.addEventListener('keydown',e=>{
  const field=editorField(e.target);if(!field)return;
  if((e.metaKey||e.ctrlKey)&&e.key.toLowerCase()==='a'){
    e.preventDefault();e.stopPropagation();
    if(field instanceof HTMLInputElement||field instanceof HTMLTextAreaElement)field.select();
    else{const r=document.createRange();r.selectNodeContents(field);const s=getSelection();s.removeAllRanges();s.addRange(r)}
    return;
  }
  if(e.key==='Home'||e.key==='End'){
    e.preventDefault();e.stopPropagation();
    if(field instanceof HTMLInputElement||field instanceof HTMLTextAreaElement){const p=e.key==='Home'?0:field.value.length;field.setSelectionRange(p,p)}
    else{const s=getSelection();if(s&&s.modify)s.modify('move',e.key==='Home'?'backward':'forward','lineboundary');else{const r=document.createRange();r.selectNodeContents(field);r.collapse(e.key==='Home');s.removeAllRanges();s.addRange(r)}}
    return;
  }
  if(e.metaKey||e.ctrlKey||e.altKey||e.isComposing)return;
  const key=e.key||'',one=Array.from(key).length===1;
  const printable=one&&!/[\u0000-\u001f\u007f-\u009f]/.test(key)&&!Array.from(key).some(privateUse);
  if(!printable&&!editorKeys.has(key))e.preventDefault();
},true);
"#
}

fn base_css(
    bg: &str,
    text: &str,
    muted: &str,
    faint: &str,
    accent: &str,
    border: &str,
    wash: &str,
) -> String {
    format!(
        r#"
*{{box-sizing:border-box}}html,body{{margin:0;min-height:100%;background:{bg};color:{text};font:14px/1.5 -apple-system,BlinkMacSystemFont,"Inter",sans-serif;-webkit-font-smoothing:antialiased}}body{{padding-bottom:48px}}header{{height:46px;display:flex;align-items:center;gap:14px;padding:0 16px;border-bottom:1px solid {border};position:sticky;top:0;background:{bg};z-index:3}}.filename{{color:{muted};font-size:12px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;flex:1}}.tools{{display:flex;gap:5px;align-items:center}}button{{font:inherit;color:{muted};background:transparent;border:1px solid {border};border-radius:6px;padding:5px 9px;cursor:pointer}}button:hover{{background:{wash};color:{text}}}button.save{{background:{text};color:{bg};border-color:{text};margin-left:5px}}main{{max-width:980px;margin:28px auto;padding:0 24px}}.page{{max-width:760px;min-height:80vh;margin:28px auto;background:{bg};border:1px solid {border};box-shadow:0 3px 22px {wash};padding:74px 82px;outline:none}}.doc-title{{font-size:30px;font-weight:650;line-height:1.25;margin:0 0 30px;outline:none}}.blocks{{min-height:300px;outline:none}}.blocks>*{{margin:0 0 13px;outline:none}}.blocks h1{{font-size:26px}}.blocks h2{{font-size:21px}}.blocks h3{{font-size:17px}}.blocks blockquote{{border-left:2px solid {border};padding-left:14px;color:{muted}}}.blocks pre{{font-family:ui-monospace,SFMono-Regular,Menlo,monospace;background:{wash};padding:10px;border-radius:6px}}.blocks table{{width:100%;border-collapse:collapse}}.blocks td{{border:1px solid {border};padding:7px}}.blocks ul,.blocks ol{{padding-left:22px}}.sheet-tabs{{display:flex;gap:4px;border-bottom:1px solid {border};margin-bottom:10px}}.sheet-tabs button.active,.rail button.active{{color:{text};background:{wash}}}.grid-wrap{{overflow:auto;border:1px solid {border};max-height:calc(100vh - 190px)}}table.grid{{border-collapse:collapse;min-width:100%;width:max-content;font:13px ui-monospace,SFMono-Regular,Menlo,monospace}}.grid th,.grid td{{border-right:1px solid {border};border-bottom:1px solid {border};min-width:112px;height:34px;padding:0}}.grid thead th,.grid .rownum{{background:{wash};color:{faint};text-align:center;font-weight:500;min-width:38px}}.grid input{{width:100%;height:100%;background:transparent;color:{text};border:0;padding:6px 8px;font:inherit;outline:none}}.grid input:focus{{box-shadow:inset 0 0 0 1px {accent}}}.ppt{{display:flex;gap:20px}}.rail{{width:150px;display:flex;flex-direction:column;gap:5px}}.rail button{{text-align:left;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}}.slide{{width:min(100%,760px);aspect-ratio:16/9;border:1px solid {border};box-shadow:0 3px 22px {wash};padding:48px 56px;display:flex;flex-direction:column;gap:20px;outline:none}}.slide-title{{font-size:28px;font-weight:650;outline:none}}.slide-bullets{{font-size:17px;line-height:1.8;outline:none;flex:1}}.notes{{display:block;width:100%;min-height:68px;margin-top:14px;padding:10px;border:1px solid {border};border-radius:6px;color:{muted};background:{wash};font:inherit;resize:vertical}}.muted{{color:{muted}}}.tag{{font-size:11px;color:{faint};margin:8px 0}}@media(max-width:700px){{.page{{padding:36px 25px}}main{{padding:0 12px}}.ppt{{gap:10px}}.rail{{width:90px}}.slide{{padding:24px}}}}
"#
    )
}

fn docx_app() -> String {
    r#"
const esc=s=>String(s??'').replace(/[&<>"]/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;'}[c]));
const marked=s=>esc(s).replace(/\*\*(.+?)\*\*/g,'<strong>$1</strong>');
const app=document.getElementById('app'),bar=document.getElementById('toolbar');
bar.innerHTML='<button data-cmd="h1">H1</button><button data-cmd="h2">H2</button><button data-cmd="p">Text</button><button data-cmd="ul">Bullet</button><button data-cmd="ol">Numbered</button><button data-cmd="bold"><b>B</b></button><button id="table">Table</button><button id="row">Add row</button><button class="save" id="save">Save</button>';
const inner=b=>{let t=b.text??'';if(b.type==='page_break')return '<hr data-type="page_break">';if(b.type==='table')return '<table>'+b.rows.map(r=>'<tr>'+r.map(c=>'<td contenteditable="true">'+marked(c)+'</td>').join('')+'</tr>').join('')+'</table>';const tag=b.type==='heading'?'h'+Math.max(1,Math.min(3,b.level||1)):({bullet:'li',numbered:'li',quote:'blockquote',code:'pre'}[b.type]||'p');return '<'+tag+' data-type="'+esc(b.type)+'" '+(b.type==='bullet'?'data-list="ul"':b.type==='numbered'?'data-list="ol"':'')+' contenteditable="true">'+marked(t)+'</'+tag+'>';};
function render(){const blocks=d.blocks||[];app.innerHTML='<section class="page"><h1 class="doc-title" id="title" contenteditable="true">'+esc(d.title)+'</h1><div class="blocks" id="blocks">'+blocks.map(inner).join('')+'</div></section>';if(!blocks.length)document.getElementById('blocks').innerHTML='<p contenteditable="true">Start writing…</p>'}
render();bar.querySelectorAll('[data-cmd]').forEach(b=>b.onclick=()=>{document.execCommand(({h1:'formatBlock',h2:'formatBlock',p:'formatBlock',ul:'insertUnorderedList',ol:'insertOrderedList',bold:'bold'})[b.dataset.cmd],false,({h1:'h1',h2:'h2',p:'p'}[b.dataset.cmd]));document.getElementById('blocks').focus()});
document.getElementById('table').onclick=()=>{document.getElementById('blocks').insertAdjacentHTML('beforeend','<table><tr><td contenteditable="true">Heading</td><td contenteditable="true">Value</td></tr><tr><td contenteditable="true"></td><td contenteditable="true"></td></tr></table>')};
document.getElementById('row').onclick=()=>{const t=document.querySelector('#blocks table');if(t){const n=t.rows[0]?.cells.length||2;const r=t.insertRow();for(let i=0;i<n;i++)r.insertCell().contentEditable='true'}};
const md=n=>{if(n.nodeType===3)return n.nodeValue;let s=[...n.childNodes].map(md).join('');if(n.nodeName==='STRONG'||n.nodeName==='B')return '**'+s+'**';if(n.nodeName==='BR')return '\n';return s};
function save(){d.title=document.getElementById('title').innerText.trim();d.blocks=[...document.getElementById('blocks').children].flatMap(el=>{if(el.dataset.type==='page_break')return [{type:'page_break'}];if(el.tagName==='TABLE')return [{type:'table',rows:[...el.rows].map(r=>[...r.cells].map(c=>md(c).trim()))}];if(el.tagName==='UL'||el.tagName==='OL')return [...el.querySelectorAll('li')].map(li=>({type:el.tagName==='UL'?'bullet':'numbered',text:md(li).trim()}));const type=el.dataset.type||el.dataset.list||(el.tagName==='H1'||el.tagName==='H2'||el.tagName==='H3'?'heading':el.tagName==='BLOCKQUOTE'?'quote':el.tagName==='PRE'?'code':'paragraph');return [type==='heading'?{type,level:Number(el.tagName.slice(1))||1,text:md(el).trim()}:{type,text:md(el).trim()}]});window.ipc.postMessage(JSON.stringify({type:'doc_save',document:d}));}
document.getElementById('save').onclick=save;document.addEventListener('keydown',e=>{if((e.metaKey||e.ctrlKey)&&e.key.toLowerCase()==='s'){e.preventDefault();save()}});
"#.into()
}

fn xlsx_app() -> String {
    r#"
const app=document.getElementById('app'),bar=document.getElementById('toolbar');let si=0;
bar.innerHTML='<button id="addrow">Add row</button><button id="addcol">Add column</button><button class="save" id="save">Save</button>';
function render(){d.sheets=d.sheets||[{name:'Sheet1',rows:[]}];si=Math.min(si,d.sheets.length-1);const s=d.sheets[si];const cols=Math.max(1,...s.rows.map(r=>r.length));const letters=n=>{let x='',v=n+1;while(v){let m=(v-1)%26;x=String.fromCharCode(65+m)+x;v=Math.floor((v-1)/26)}return x};
app.innerHTML='<nav class="sheet-tabs">'+d.sheets.map((x,i)=>'<button class="'+(i===si?'active':'')+'" data-i="'+i+'">'+String(x.name).replace(/[&<>]/g,'')+'</button>').join('')+'</nav><div class="grid-wrap"><table class="grid"><thead><tr><th></th>'+Array.from({length:cols},(_,c)=>'<th>'+letters(c)+'</th>').join('')+'</tr></thead><tbody>'+s.rows.map((row,r)=>'<tr><th class="rownum">'+(r+1)+'</th>'+Array.from({length:cols},(_,c)=>'<td><input data-r="'+r+'" data-c="'+c+'" value="'+String(row[c]??'').replace(/&/g,'&amp;').replace(/"/g,'&quot;').replace(/</g,'&lt;')+'"></td>').join('')+'</tr>').join('')+'</tbody></table></div>';
app.querySelectorAll('[data-i]').forEach(b=>b.onclick=()=>{capture();si=Number(b.dataset.i);render()})}
function capture(){const s=d.sheets[si];document.querySelectorAll('.grid input').forEach(i=>{const r=Number(i.dataset.r),c=Number(i.dataset.c);s.rows[r]=s.rows[r]||[];const v=i.value;if(v.startsWith('='))s.rows[r][c]={text:v};else if(v.trim()!==''&&!Number.isNaN(Number(v)))s.rows[r][c]={number:Number(v)};else s.rows[r][c]={text:v}});s.rows=s.rows.map(row=>row.map(c=>c&&typeof c==='object'&&'number'in c?c.number:c&&typeof c==='object'?c.text??'':c))}
render();document.getElementById('addrow').onclick=()=>{capture();d.sheets[si].rows.push([]);render()};document.getElementById('addcol').onclick=()=>{capture();for(const row of d.sheets[si].rows)row.push('');render()};
function save(){capture();window.ipc.postMessage(JSON.stringify({type:'doc_save',document:d}))}document.getElementById('save').onclick=save;document.addEventListener('keydown',e=>{if((e.metaKey||e.ctrlKey)&&e.key.toLowerCase()==='s'){e.preventDefault();save()}});
"#.into()
}

fn pptx_app() -> String {
    r#"
const app=document.getElementById('app'),bar=document.getElementById('toolbar');d.slides=d.slides||[];let ix=0;
bar.innerHTML='<button id="add">Add slide</button><button id="up">Move up</button><button id="down">Move down</button><button id="del">Delete slide</button><button class="save" id="save">Save</button>';
const esc=s=>String(s??'').replace(/[&<>"]/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;'}[c]));
function render(){if(!d.slides.length)d.slides.push({title:'',bullets:[],notes:''});ix=Math.min(ix,d.slides.length-1);const s=d.slides[ix];app.innerHTML='<div class="ppt"><nav class="rail">'+d.slides.map((x,i)=>'<button data-i="'+i+'" class="'+(ix===i?'active':'')+'">'+(i+1)+'. '+esc(x.title||'Untitled')+'</button>').join('')+'</nav><div style="flex:1"><section class="slide"><div class="slide-title" id="title" contenteditable="true">'+esc(s.title)+'</div><div class="slide-bullets" id="bullets" contenteditable="true">'+(s.bullets||[]).map(x=>'• '+esc(x)).join('<br>')+'</div></section><textarea class="notes" id="notes" placeholder="Speaker notes">'+esc(s.notes)+'</textarea></div></div>';app.querySelectorAll('[data-i]').forEach(b=>b.onclick=()=>{capture();ix=Number(b.dataset.i);render()})}
function capture(){const s=d.slides[ix];s.title=document.getElementById('title').innerText.trim();s.bullets=document.getElementById('bullets').innerText.split('\n').map(x=>x.replace(/^[•*-]\s*/, '').trim()).filter(Boolean);s.notes=document.getElementById('notes').value}
render();document.getElementById('add').onclick=()=>{capture();d.slides.splice(ix+1,0,{title:'New slide',bullets:[],notes:''});ix++;render()};document.getElementById('del').onclick=()=>{if(d.slides.length>1){d.slides.splice(ix,1);ix=Math.max(0,ix-1);render()}};document.getElementById('up').onclick=()=>{if(ix>0){capture();[d.slides[ix-1],d.slides[ix]]=[d.slides[ix],d.slides[ix-1]];ix--;render()}};document.getElementById('down').onclick=()=>{if(ix<d.slides.length-1){capture();[d.slides[ix+1],d.slides[ix]]=[d.slides[ix],d.slides[ix+1]];ix++;render()}};function save(){capture();window.ipc.postMessage(JSON.stringify({type:'doc_save',document:d}))}document.getElementById('save').onclick=save;document.addEventListener('keydown',e=>{if((e.metaKey||e.ctrlKey)&&e.key.toLowerCase()==='s'){e.preventDefault();save()}});
"#.into()
}

fn block_plain(block: &Block) -> String {
    match block {
        Block::Heading { text, .. }
        | Block::Paragraph { text }
        | Block::Bullet { text }
        | Block::Numbered { text }
        | Block::Quote { text }
        | Block::Code { text } => text.clone(),
        Block::Table { rows } => rows
            .iter()
            .map(|r| r.join(" | "))
            .collect::<Vec<_>>()
            .join("\n"),
        Block::PageBreak => String::new(),
    }
}

fn html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
