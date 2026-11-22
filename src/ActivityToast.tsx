import {useEffect,useState} from 'react';
import type {Activity} from './ActivityDialog';
import type {NotificationLocation} from './uiPreferences';
import {X} from './OriginalIcons';
import './activity-toast.css';
export function ActivityToast({event,location,showMessages,onOpen}:{event:Activity|undefined;location:NotificationLocation;showMessages:boolean;onOpen:()=>void}){
 const [dismissed,setDismissed]=useState<number|null>(null),[hovered,setHovered]=useState(false),[focused,setFocused]=useState(false),[modal,setModal]=useState(false);
 useEffect(()=>{const sync=()=>setModal(!!document.querySelector('[aria-modal="true"]'));sync();const observer=new MutationObserver(sync);observer.observe(document.body,{childList:true,subtree:true,attributes:true,attributeFilter:['aria-modal']});return()=>observer.disconnect();},[]);
 useEffect(()=>{setHovered(false);setFocused(false);},[event?.id,modal]);
 useEffect(()=>{if(!event||event.kind==='error'||hovered||focused)return;const timer=setTimeout(()=>setDismissed(event.id),8000);return()=>clearTimeout(timer);},[event?.id,event?.kind,hovered,focused]);
 if(!event||dismissed===event.id||modal||(event.kind==='notice'&&!showMessages))return null;
 return <aside className={`activity-toast ${location} ${event.kind}`} aria-label="Latest activity notification" onMouseEnter={()=>setHovered(true)} onMouseLeave={()=>setHovered(false)} onFocusCapture={()=>setFocused(true)} onBlurCapture={event=>{if(!event.currentTarget.contains(event.relatedTarget as Node|null))setFocused(false);}}><header><strong>{event.kind==='error'?'Operation failed':'Activity'}</strong><button aria-label="Dismiss activity notification" title="Dismiss notification" onClick={()=>setDismissed(event.id)}><X size={16}/></button></header><p role="status">{event.message}</p><button onClick={()=>{setDismissed(event.id);onOpen();}}>View activity</button></aside>;
}
