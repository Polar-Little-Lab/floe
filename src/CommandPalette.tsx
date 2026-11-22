import {useEffect,useId,useRef,useState} from 'react';
import type {KeyboardEvent} from 'react';
import {X} from './OriginalIcons';
export type Command={label:string;keywords?:string;disabled?:boolean;action:()=>void};
export function CommandPalette({commands,onClose}:{commands:Command[];onClose:()=>void}){
 const [query,setQuery]=useState('');const [active,setActive]=useState('');const id=useId();const list=useRef<HTMLDivElement>(null);
 const terms=query.toLowerCase().trim().split(/\s+/).filter(Boolean);
 const matches=commands.filter(command=>terms.every(term=>`${command.label} ${command.keywords??''}`.toLowerCase().includes(term)));
 const enabled=matches.filter(command=>!command.disabled);
 const selected=enabled.find(command=>command.label===active)??enabled[0];
 const selectedIndex=matches.findIndex(command=>command===selected);
 useEffect(()=>{list.current?.querySelector('[aria-selected="true"]')?.scrollIntoView({block:'nearest'});},[selected?.label]);
 const execute=(command:Command)=>{if(command.disabled)return;onClose();command.action();};
 function key(event:KeyboardEvent<HTMLInputElement>){
  if(event.nativeEvent.isComposing)return;
  if(event.key==='Enter'){event.preventDefault();if(selected)execute(selected);return;}
  if(!['ArrowDown','ArrowUp','Home','End'].includes(event.key))return;
  if((event.key==='Home'||event.key==='End')&&!event.altKey)return;
  event.preventDefault();if(!enabled.length)return;
  const index=enabled.indexOf(selected);
  const next=event.key==='Home'?0:event.key==='End'?enabled.length-1:event.key==='ArrowDown'?(index+1)%enabled.length:(index-1+enabled.length)%enabled.length;
  setActive(enabled[next].label);
 }
 return <div className="modal-backdrop"><section className="modal command-palette" role="dialog" aria-modal="true" aria-label="Command palette">
  <div className="modal-header"><h2>Commands</h2><button data-modal-close aria-label="Close command palette" onClick={onClose}><X size={18}/></button></div>
  <input autoFocus className="wide-input" role="combobox" aria-label="Search commands" aria-expanded="true" aria-controls={id} aria-activedescendant={selectedIndex>=0?`${id}-${selectedIndex}`:undefined} aria-autocomplete="list" placeholder="Search commands…" maxLength={200} value={query} onChange={event=>{setQuery(event.target.value);setActive('');}} onKeyDown={key}/>
  <div className="command-results" role="listbox" aria-label="Commands" id={id} ref={list}>{matches.map((command,index)=><button id={`${id}-${index}`} key={command.label} role="option" aria-selected={command===selected} aria-disabled={!!command.disabled} disabled={command.disabled} onClick={()=>execute(command)}><span>{command.label}</span>{command.disabled&&<small>Unavailable</small>}</button>)}</div>
  {!matches.length&&<p role="status">No matching commands.</p>}
  <small>↑ ↓ Select · Enter Run · Escape Close · ⌘/Ctrl Shift P Open</small>
 </section></div>;
}
