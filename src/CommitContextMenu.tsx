import type {KeyboardEvent} from 'react';
export type CommitMenuItem={label:string;disabled?:boolean;action:()=>void};
export function CommitContextMenu({title,x,y,items,onClose,label='Commit actions'}:{title:string;x:number;y:number;items:CommitMenuItem[];onClose:()=>void;label?:string}) {
 function navigate(event:KeyboardEvent<HTMLDivElement>) {
  if(!['ArrowDown','ArrowUp','Home','End'].includes(event.key))return;
  event.preventDefault();
  const buttons=Array.from(event.currentTarget.querySelectorAll<HTMLButtonElement>('[role="menuitem"]:not(:disabled)'));
  const current=buttons.indexOf(document.activeElement as HTMLButtonElement);
  const next=event.key==='Home'?0:event.key==='End'?buttons.length-1:event.key==='ArrowDown'?(current+1)%buttons.length:(current-1+buttons.length)%buttons.length;
  buttons[next]?.focus();
 }
 return <div className="commit-context-backdrop" onMouseDown={event=>{if(event.target===event.currentTarget)onClose();}} onContextMenu={event=>{event.preventDefault();onClose();}}>
  <section className="commit-context-menu" role="dialog" aria-modal="true" aria-label={label} style={{left:x,top:y}} onKeyDown={event=>{if(event.key==='Escape'){event.preventDefault();event.stopPropagation();onClose();}}}>
   <div className="commit-context-heading" title={title}>{title}</div>
   <div role="menu" aria-label={label} onKeyDown={navigate}>{items.map(item=><button role="menuitem" key={item.label} disabled={item.disabled} onClick={()=>{onClose();item.action();}}>{item.label}</button>)}</div>
   <button className="commit-context-close" data-modal-close onClick={onClose}>Close</button>
  </section>
 </div>;
}
