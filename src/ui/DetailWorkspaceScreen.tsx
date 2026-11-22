import {useState,type ReactNode} from 'react';
import {WorkspaceScreen} from './WorkspaceScreen';

/** Launchpad master/detail surface; expanding preserves the mounted editor and drafts. */
export function DetailWorkspaceScreen({label,title,icon,busy,onClose,children}:{label:string;title:string;icon:ReactNode;busy:boolean;onClose:()=>void;children:ReactNode}){
 const [expanded,setExpanded]=useState(false);
 return <WorkspaceScreen modal={expanded} label={label} title={title} icon={icon} busy={busy} onClose={onClose} closeLabel="Close provider panel"
  className={`provider-item-screen detail-workspace-screen${expanded?' detail-expanded':' detail-docked'}`}
  actions={<button aria-pressed={expanded} onClick={()=>setExpanded(value=>!value)}>{expanded?'Show list beside detail':'Expand detail'}</button>}>
  {children}
 </WorkspaceScreen>;
}
