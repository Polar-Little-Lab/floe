import {useEffect,useId,useRef,useState} from 'react';
import {ChevronDown} from '../OriginalIcons';
export interface ToolbarAction{label:string;disabled?:boolean;title?:string;onSelect?:()=>void}
export function ToolbarActionMenu({label,disabled,actions}:{label:string;disabled:boolean;actions:readonly ToolbarAction[]}){
 const [open,setOpen]=useState(false);const root=useRef<HTMLDivElement>(null),trigger=useRef<HTMLButtonElement>(null);const id=useId();
 const close=(focus=false)=>{setOpen(false);if(focus)trigger.current?.focus();};
 useEffect(()=>{if(disabled)setOpen(false);},[disabled]);
 useEffect(()=>{if(!open)return;root.current?.querySelector<HTMLButtonElement>('[role=menuitem]:not(:disabled)')?.focus();const outside=(event:PointerEvent)=>{if(!root.current?.contains(event.target as Node))setOpen(false);};document.addEventListener('pointerdown',outside);return()=>document.removeEventListener('pointerdown',outside);},[open]);
 return <div ref={root} className="toolbar-action-menu" onBlur={event=>{if(!root.current?.contains(event.relatedTarget as Node|null))setOpen(false);}} onKeyDown={event=>{
  if(event.key==='Escape'){event.preventDefault();event.stopPropagation();close(true);}
  if(!open||!['ArrowDown','ArrowUp','Home','End'].includes(event.key))return;
  event.preventDefault();const items=[...(root.current?.querySelectorAll<HTMLButtonElement>('[role=menuitem]:not(:disabled)')??[])];if(!items.length)return;const index=items.indexOf(document.activeElement as HTMLButtonElement);const next=event.key==='Home'?0:event.key==='End'?items.length-1:(index+(event.key==='ArrowDown'?1:-1)+items.length)%items.length;items[next]?.focus();
 }}><button ref={trigger} disabled={disabled} aria-haspopup="menu" aria-expanded={open} aria-controls={open?id:undefined} onClick={()=>setOpen(!open)} onKeyDown={event=>{if(!open&&['ArrowDown','ArrowUp'].includes(event.key)){event.preventDefault();setOpen(true);}}}>{label}<ChevronDown size={14}/></button>{open&&<div id={id} role="menu" aria-label={label}>{actions.map(action=><button key={action.label} type="button" role="menuitem" disabled={action.disabled} title={action.title} onClick={()=>{close(true);action.onSelect?.();}}>{action.label}</button>)}</div>}</div>;
}
