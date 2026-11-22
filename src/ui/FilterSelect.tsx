import {useEffect,useId,useRef,useState,type ReactNode,type KeyboardEvent} from 'react';
import {ChevronDown,Check,Search} from '../OriginalIcons';
export type FilterOption={value:string;label:string;icon?:ReactNode};
/** Compact reference-style filter; persistence and navigation guards stay with its caller. */
export function FilterSelect({label,value,options,placeholder,disabled=false,onChange,icon,searchable=false}:{label:string;value:string;options:readonly FilterOption[];placeholder:string;icon?:ReactNode;disabled?:boolean;searchable?:boolean;onChange:(value:string)=>void}){
 const [open,setOpen]=useState(false),[active,setActive]=useState(0),[query,setQuery]=useState('');
 const root=useRef<HTMLDivElement>(null),trigger=useRef<HTMLButtonElement>(null),list=useRef<HTMLDivElement>(null),search=useRef<HTMLInputElement>(null);
 const id=useId();const selected=options.find(option=>option.value===value);
 const terms=query.trim().toLocaleLowerCase().split(/\s+/).filter(Boolean);
 const visible=searchable&&terms.length?options.filter(option=>terms.every(term=>option.label.toLocaleLowerCase().includes(term))):options;
 const activeId=visible[active]?`${id}-${active}`:undefined;
 const close=(focus=false)=>{setOpen(false);if(focus)trigger.current?.focus();};
 const show=(last=false)=>{if(disabled||!options.length)return;setQuery('');setActive(last?options.length-1:Math.max(0,options.findIndex(option=>option.value===value)));setOpen(true);};
 useEffect(()=>{if(disabled)setOpen(false);},[disabled]);
 useEffect(()=>{if(open)(searchable?search.current:list.current)?.focus();},[open,searchable]);
 useEffect(()=>{if(open){const index=Math.min(active,Math.max(0,visible.length-1));if(index!==active)setActive(index);list.current?.querySelectorAll('[role=option]')[index]?.scrollIntoView({block:'nearest'});}},[open,active,visible.length,query]);
 useEffect(()=>{if(!open)return;const outside=(event:PointerEvent)=>{if(!root.current?.contains(event.target as Node))setOpen(false);};document.addEventListener('pointerdown',outside);return()=>document.removeEventListener('pointerdown',outside);},[open]);
 const choose=(index:number)=>{const option=visible[index];if(disabled||!option)return;close(true);if(option.value!==value)onChange(option.value);};
 const navigate=(event:KeyboardEvent<HTMLElement>)=>{
  if(event.nativeEvent.isComposing)return;
  const editing=event.target===search.current;
  if(event.key==='Escape'){event.preventDefault();event.stopPropagation();close(true);}
  else if(event.key==='ArrowDown'||event.key==='ArrowUp'){event.preventDefault();setActive(index=>(index+(event.key==='ArrowDown'?1:-1)+visible.length)%Math.max(1,visible.length));}
  else if(!editing&&(event.key==='Home'||event.key==='End')){event.preventDefault();setActive(event.key==='Home'?0:Math.max(0,visible.length-1));}
  else if(event.key==='Enter'||(!editing&&event.key===' ')){event.preventDefault();event.stopPropagation();choose(active);}
  else if(!editing&&event.key.length===1&&!event.ctrlKey&&!event.metaKey&&!event.altKey){const key=event.key.toLocaleLowerCase();const order=[...visible.slice(active+1),...visible.slice(0,active+1)];const match=order.find(option=>option.label.toLocaleLowerCase().startsWith(key));if(match){event.preventDefault();setActive(visible.indexOf(match));}}
 };
 return <div ref={root} className="filter-select" onBlur={event=>{if(!event.currentTarget.contains(event.relatedTarget as Node|null))setOpen(false);}}>
  <button ref={trigger} type="button" disabled={disabled} aria-label={`${label}: ${selected?.label??placeholder}`} aria-haspopup="listbox" aria-expanded={open} aria-controls={open?id:undefined} onClick={()=>open?close():show()} onKeyDown={event=>{if(event.key==='ArrowDown'||event.key==='ArrowUp'){event.preventDefault();show(event.key==='ArrowUp');}}}>{icon??selected?.icon}<span>{value?selected?.label??placeholder:placeholder}</span><ChevronDown size={14}/></button>
  {open&&<div className="filter-select-popup">
   {searchable&&<label className="filter-select-search"><Search size={15}/><input ref={search} role="combobox" aria-label={`Search ${label.toLowerCase()} options`} aria-autocomplete="list" aria-expanded="true" aria-controls={id} aria-activedescendant={activeId} placeholder="Search" maxLength={4096} value={query} onChange={event=>{setQuery(event.target.value);setActive(0);}} onKeyDown={navigate}/></label>}
   <div ref={list} id={id} className="filter-select-options" role="listbox" tabIndex={searchable?undefined:-1} aria-label={label} aria-activedescendant={searchable?undefined:activeId} onKeyDown={navigate}>{visible.map((option,index)=><div key={option.value} id={`${id}-${index}`} role="option" aria-selected={value===option.value} className={active===index?'active':''} onPointerMove={()=>setActive(index)} onMouseDown={event=>event.preventDefault()} onClick={()=>choose(index)}>{option.icon}<span>{option.label}</span>{value===option.value&&<Check size={13}/>}</div>)}</div>
   {!visible.length&&<p className="filter-select-empty" role="status">No matching options.</p>}
  </div>}
 </div>;
}
