import {useId,useState,type ReactNode} from 'react';
import {ChevronDown,ChevronRight,Folder} from '../OriginalIcons';
/** Folder navigation shared by reference trees; filtering exposes all matching rows. */
export function ReferenceFolder({label,count,filtering=false,children}:{label:string;count:number;filtering?:boolean;children:ReactNode}){
 const [expanded,setExpanded]=useState(true);
 const id=useId(),visible=filtering||expanded;
 return <div className="reference-folder" hidden={count===0}><button className="branch-item reference-folder-heading" aria-expanded={visible} aria-controls={id} aria-label={`${label}, ${count} branches`} disabled={filtering} title={filtering?'Matching branches stay expanded while filtering':label} onClick={()=>setExpanded(!expanded)}>{visible?<ChevronDown size={12}/>:<ChevronRight size={12}/>}<Folder size={13}/><span>{label}</span><small>{count}</small></button><div id={id} className="reference-folder-children" hidden={!visible}>{children}</div></div>;
}
