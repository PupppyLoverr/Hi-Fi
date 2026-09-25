import Foundation
import HiFiCore

/// JavaScript snippets for the agent browser-use layer. No CDP, no Chromium —
/// everything runs inside the tab's WKWebView.
enum JSBridge {

    /// Walk the DOM, tag interactive/structural elements with @eN refs,
    /// return a compact JSON tree.
    static let snapshot = """
    (function(){
      var refs = {};
      var counter = 0;
      var nodes = [];
      function visible(el){
        var r = el.getBoundingClientRect();
        var s = getComputedStyle(el);
        return r.width>0 && r.height>0 && s.visibility!=='hidden'
            && s.display!=='none' && r.bottom>-50 && r.right>0
            && r.top<innerHeight*2 && r.left<innerWidth*2;
      }
      function name(el){
        return (el.getAttribute('aria-label')||el.innerText||el.value
             ||el.placeholder||el.title||el.alt||'').replace(/\\s+/g,' ').trim().slice(0,80);
      }
      var roleMap={a:'link',button:'button',input:'textbox',textarea:'textbox',
        select:'combobox',h1:'heading',h2:'heading',h3:'heading',h4:'heading',
        h5:'heading',h6:'heading',img:'img',nav:'navigation',main:'main',
        form:'form',summary:'button',label:'label',li:'listitem',ul:'list',
        ol:'list',table:'table',dialog:'dialog'};
      function role(el){
        var r=el.getAttribute('role'); if(r) return r;
        return roleMap[el.tagName.toLowerCase()]||el.tagName.toLowerCase();
      }
      var sel='a,button,input,textarea,select,[role],[onclick],[data-hifi-ref],'
             +'h1,h2,h3,h4,h5,h6,summary,label,[tabindex]';
      var all=document.querySelectorAll(sel);
      for(var i=0;i<all.length;i++){
        var el=all[i];
        if(!visible(el)) continue;
        var ref=el.getAttribute('data-hifi-ref');
        if(!ref){ ref='@e'+(++counter); el.setAttribute('data-hifi-ref',ref); }
        var r=el.getBoundingClientRect();
        nodes.push({ref:ref,role:role(el),name:name(el),tag:el.tagName.toLowerCase(),
          href:el.href||null,value:(el.value!==undefined&&el.value!==''?el.value:null),
          rect:[Math.round(r.x),Math.round(r.y),Math.round(r.width),Math.round(r.height)]});
        if(nodes.length>=400) break;
      }
      return JSON.stringify({url:location.href,title:document.title,nodes:nodes});
    })()
    """

    static func findElement(target: String) -> String {
        if target.hasPrefix("@e") {
            return "document.querySelector('[data-hifi-ref=\"\(target)\"]')"
        }
        return "document.querySelector(\(target.asJSString))"
    }

    static func click(target: String) -> String {
        """
        (function(){
          var el = \(findElement(target: target));
          if(!el) return 'ERR:not-found';
          el.scrollIntoView({block:'center',inline:'center'});
          ['mouseover','mousedown','mouseup','click'].forEach(function(ty){
            el.dispatchEvent(new MouseEvent(ty,{bubbles:true,cancelable:true,view:window}));
          });
          return 'ok';
        })()
        """
    }

    static func typeText(target: String, text: String) -> String {
        """
        (function(){
          var el = \(findElement(target: target));
          if(!el) return 'ERR:not-found';
          el.scrollIntoView({block:'center',inline:'center'});
          el.focus();
          var proto = el instanceof HTMLTextAreaElement
                      ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
          var desc = Object.getOwnPropertyDescriptor(proto,'value');
          var text = \(text.asJSString);
          if(desc && desc.set){ desc.set.call(el,text); } else { el.value = text; }
          el.dispatchEvent(new Event('input',{bubbles:true}));
          el.dispatchEvent(new Event('change',{bubbles:true}));
          return 'ok';
        })()
        """
    }

    static func press(key: String) -> String {
        """
        (function(){
          var map={Enter:{key:'Enter',code:'Enter',keyCode:13},
            Tab:{key:'Tab',code:'Tab',keyCode:9},
            Escape:{key:'Escape',code:'Escape',keyCode:27},
            Backspace:{key:'Backspace',code:'Backspace',keyCode:8},
            Delete:{key:'Delete',code:'Delete',keyCode:46},
            ArrowDown:{key:'ArrowDown',code:'ArrowDown',keyCode:40},
            ArrowUp:{key:'ArrowUp',code:'ArrowUp',keyCode:38},
            ArrowLeft:{key:'ArrowLeft',code:'ArrowLeft',keyCode:37},
            ArrowRight:{key:'ArrowRight',code:'ArrowRight',keyCode:39},
            Space:{key:' ',code:'Space',keyCode:32}};
          var k = \(key.asJSString);
          var info = map[k] || {key:k,code:'Key'+k.toUpperCase(),keyCode:k.charCodeAt(0)};
          var el = document.activeElement || document.body;
          ['keydown','keypress','keyup'].forEach(function(ty){
            el.dispatchEvent(new KeyboardEvent(ty,{key:info.key,code:info.code,
              keyCode:info.keyCode,which:info.keyCode,bubbles:true,cancelable:true}));
          });
          return 'ok';
        })()
        """
    }

    static let currentURL = "location.href"
    static let pageHTML   = "document.documentElement.outerHTML"
    static let pageTitle  = "document.title"
}
