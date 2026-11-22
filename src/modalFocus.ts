// Shared DOM modal behavior. AppKit dialogs remain owned by the native platform.
const MODAL='[role="dialog"][aria-modal="true"],[role="alertdialog"][aria-modal="true"]';
const CONTROLS='button,input,select,textarea,a[href],summary,[tabindex],[contenteditable="true"]';
function visible(element:HTMLElement):boolean {
 for(let parent=element.parentElement;parent;parent=parent.parentElement){
  if(parent instanceof HTMLDetailsElement&&!parent.open){const summary=Array.from(parent.children).find(child=>child.tagName==='SUMMARY');if(!summary?.contains(element))return false;}
 }
 const style=getComputedStyle(element);
 return element.isConnected&&!element.closest('[hidden]')&&style.visibility!=='hidden'&&style.visibility!=='collapse'&&element.getClientRects().length>0;
}
function available(element:HTMLElement):boolean {
 return visible(element)&&!element.closest('[inert]')&&!element.matches(':disabled')&&element.getAttribute('aria-disabled')!=='true';
}
function tabIndex(element:HTMLElement):number {return element.isContentEditable&&!element.hasAttribute('tabindex')?0:element.tabIndex;}
export function modalTabStops(modal:HTMLElement):HTMLElement[] {
 const all=Array.from(modal.querySelectorAll<HTMLElement>(CONTROLS)).filter(element=>available(element)&&tabIndex(element)>=0);
 const controls=all.filter(element=>{
  if(!(element instanceof HTMLInputElement)||element.type!=='radio'||!element.name)return true;
  const group=all.filter((item):item is HTMLInputElement=>item instanceof HTMLInputElement&&item.type==='radio'&&item.name===element.name&&item.form===element.form);
  return element===(group.find(item=>item.checked)??group[0]);
 });
 return controls.sort((a,b)=>{
  const aa=tabIndex(a)>0?tabIndex(a):Infinity,bb=tabIndex(b)>0?tabIndex(b):Infinity;
  return aa===bb?0:aa-bb;
 });
}
export function installModalFocus(root:HTMLElement,canDismiss:()=>boolean=()=>true):()=>void {
 const doc=root.ownerDocument;
 const masked=new Set<HTMLElement>();const addedTabIndex=new Set<HTMLElement>();
 const stack:{modal:HTMLElement;returnTo:HTMLElement|null}[]=[];
 let lastFocus=doc.activeElement instanceof HTMLElement?doc.activeElement:null;
 let active:HTMLElement|null=null,changing=false,disposed=false;
 let busyRestore:{modal:HTMLElement;control:HTMLElement}|null=null;
 const top=()=>Array.from(root.querySelectorAll<HTMLElement>(MODAL)).filter(element=>visible(element)&&!Array.from(root.querySelectorAll<HTMLElement>('[inert]')).some(parent=>!masked.has(parent)&&parent.contains(element))).at(-1)??null;
 const mask=(modal:HTMLElement|null)=>{
  const wanted=new Set<HTMLElement>();
  if(modal){let current:HTMLElement=modal;while(current!==root&&current.parentElement){for(const sibling of current.parentElement.children)if(sibling!==current&&sibling instanceof HTMLElement)wanted.add(sibling);current=current.parentElement;}}
  for(const element of masked)if(!wanted.has(element)){element.inert=false;masked.delete(element);}
  for(const element of wanted)if(!element.inert){element.inert=true;masked.add(element);}
 };
 const focusInside=(modal:HTMLElement)=>{
  const target=modalTabStops(modal)[0];
  if(target){target.focus({preventScroll:true});return;}
  if(!modal.hasAttribute('tabindex')){modal.tabIndex=-1;addedTabIndex.add(modal);}
  modal.focus({preventScroll:true});
 };
 const reconcile=()=>{
  if(disposed||changing)return;changing=true;
  try {
   for(const modal of addedTabIndex)if(!root.contains(modal)||!visible(modal)){if(modal.getAttribute('tabindex')==='-1')modal.removeAttribute('tabindex');addedTabIndex.delete(modal);}
   const next=top();let restore:HTMLElement|null=null;
   if(next!==active){
    busyRestore=null;
    const known=stack.findIndex(entry=>entry.modal===next);
    if(known>=0){while(stack.length>known+1)restore=stack.pop()!.returnTo;}
    else if(next){
     // A modal may replace another in one React commit. Preserve the original
     // invoker rather than a now-detached control from the replaced dialog.
     while(stack.length&&!stack.at(-1)!.modal.isConnected)restore=stack.pop()!.returnTo;
     stack.push({modal:next,returnTo:lastFocus?.isConnected?lastFocus:restore});
    }else {while(stack.length)restore=stack.pop()!.returnTo;}
    active=next;mask(active);
    const restorable=restore&&restore.matches(CONTROLS)&&available(restore);
    if(restore&&restorable&&(!active||active.contains(restore)))restore.focus({preventScroll:true});
    if(!active&&!restorable)modalTabStops(root)[0]?.focus({preventScroll:true});
   }else mask(active);
   if(active&&!canDismiss()&&!busyRestore&&lastFocus&&active.contains(lastFocus)&&visible(lastFocus)&&lastFocus.matches(':disabled')){
    busyRestore={modal:active,control:lastFocus};
    if(!active.hasAttribute('tabindex')){active.tabIndex=-1;addedTabIndex.add(active);}
    active.focus({preventScroll:true});
   }
   if(busyRestore&&canDismiss()){
    const pending=busyRestore;busyRestore=null;
    if(active===pending.modal&&available(pending.control)&&(doc.activeElement===active||doc.activeElement===pending.control))pending.control.focus({preventScroll:true});
   }
   if(active&&(!(doc.activeElement instanceof HTMLElement)||!active.contains(doc.activeElement)||!available(doc.activeElement))){
    if(busyRestore&&!canDismiss()){
     if(!active.hasAttribute('tabindex')){active.tabIndex=-1;addedTabIndex.add(active);}
     active.focus({preventScroll:true});
    }else focusInside(active);
   }
   if(doc.activeElement instanceof HTMLElement)lastFocus=doc.activeElement;
  } finally {changing=false;}
 };
 const focus=(event:FocusEvent)=>{
  if(changing)return;
  // Remember the invoker before React autofocus moves into a newly added dialog.
  const target=event.target;
  if(target instanceof HTMLElement&&!target.closest(MODAL)&&!active)lastFocus=target;
  reconcile();
  if(target instanceof HTMLElement&&active?.contains(target))lastFocus=target;
 };
 const key=(event:KeyboardEvent)=>{
  if(event.isComposing)return;reconcile();if(!active)return;
  if(event.key==='Tab'){
   busyRestore=null;
   event.preventDefault();event.stopImmediatePropagation();
   const controls=modalTabStops(active),index=controls.indexOf(doc.activeElement as HTMLElement);
   if(!controls.length){focusInside(active);return;}
   const next=index<0?(event.shiftKey?controls.length-1:0):(index+(event.shiftKey?-1:1)+controls.length)%controls.length;
   controls[next].focus();
  }else if(event.key==='Escape'){
   event.preventDefault();event.stopImmediatePropagation();
   if(canDismiss())Array.from(active.querySelectorAll<HTMLElement>('[data-modal-close]')).find(available)?.click();
  }else if((event.metaKey||event.ctrlKey)&&event.key.toLowerCase()==='k'){
   event.preventDefault();event.stopImmediatePropagation();
  }
 };
 const pointer=(event:PointerEvent)=>{
  if(event.target instanceof Element){const target=event.target.closest<HTMLElement>(CONTROLS);if(target&&root.contains(target)&&available(target)&&(!active||active.contains(target))){lastFocus=target;busyRestore=null;}}
 };
 doc.addEventListener('focusin',focus,true);doc.addEventListener('keydown',key,true);doc.addEventListener('pointerdown',pointer,true);
 const observer=new MutationObserver(reconcile);
 observer.observe(root,{childList:true,subtree:true,attributes:true,attributeFilter:['disabled','hidden','style','class','tabindex','open','aria-disabled','role','aria-modal']});
 reconcile();
 return()=>{disposed=true;observer.disconnect();doc.removeEventListener('focusin',focus,true);doc.removeEventListener('keydown',key,true);doc.removeEventListener('pointerdown',pointer,true);mask(null);for(const modal of addedTabIndex)if(modal.getAttribute('tabindex')==='-1')modal.removeAttribute('tabindex');};
}
