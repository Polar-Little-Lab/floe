import {useState,type ReactNode} from 'react';
import {CommitContextMenu,type CommitMenuItem} from '../CommitContextMenu';
import {Check} from '../OriginalIcons';
import './reference-row.css';
export function ReferenceRow({label,description,accessibleLabel,icon,current=false,disabled=false,busy=false,title,onSelect,onMenu,actions=[],menuLabel}:{label:string;description?:string;accessibleLabel?:string;icon:ReactNode;current?:boolean;disabled?:boolean;busy?:boolean;title?:string;onSelect:()=>void;onMenu?:(point:{x:number;y:number})=>void;actions?:CommitMenuItem[];menuLabel:string}){
 const [menu,setMenu]=useState<{x:number;y:number}|null>(null);
 const open=(x:number,y:number)=>{if(busy)return;const point={x:Math.max(8,Math.min(x,window.innerWidth-308)),y:Math.max(8,Math.min(y,window.innerHeight-Math.min(480,actions.length*32+80)))};if(onMenu)onMenu(point);else setMenu(point);};
 return <div className={`reference-item${current?' current':''}`} onContextMenu={event=>{if(event.target instanceof Element&&event.currentTarget.querySelector('[role="dialog"]')?.contains(event.target))return;event.preventDefault();open(event.clientX,event.clientY);}} onKeyDown={event=>{if(event.key==='ContextMenu'||(event.shiftKey&&event.key==='F10')){event.preventDefault();const rect=event.currentTarget.getBoundingClientRect();open(rect.right,rect.top);}}}>
  <button className="reference-item-select" disabled={disabled} title={title??label} aria-label={accessibleLabel} onClick={onSelect}><span className="reference-current">{current&&<Check size={12}/>}</span>{icon}<span className="reference-item-name">{label}{description&&<small className="reference-item-description">{description}</small>}</span></button>
  <button className="reference-item-more" disabled={busy} aria-label={menuLabel} aria-haspopup="dialog" onClick={event=>{const rect=event.currentTarget.getBoundingClientRect();open(rect.right,rect.top);}}>⋮</button>
  {menu&&<CommitContextMenu title={label} label={menuLabel} {...menu} items={actions.map(item=>({...item,disabled:busy||item.disabled}))} onClose={()=>setMenu(null)}/>}
 </div>;
}
