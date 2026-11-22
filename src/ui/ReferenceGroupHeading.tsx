import type {ReactNode} from 'react';
import {ChevronDown,ChevronRight} from '../OriginalIcons';
export function ReferenceGroupHeading({label,icon,expanded,count,busy,onToggle,onOpen}:{label:string;icon:ReactNode;expanded:boolean;count?:number;busy:boolean;onToggle:()=>void;onOpen:()=>void}){
 return <div className="reference-group-heading"><button className="side-group" aria-expanded={expanded} onClick={onToggle}>{expanded?<ChevronDown size={13}/>:<ChevronRight size={13}/>} {icon}<strong>{label}</strong>{count!==undefined&&<b>{count}</b>}</button><button aria-label={`Open ${label}`} title={`Open ${label}`} disabled={busy} onClick={onOpen}><ChevronRight size={15}/></button></div>;
}
