import {TaskGroup} from './TaskGroup';
import {useState} from 'react';
import {GitBranch} from '../OriginalIcons';
import {ReferenceRow} from './ReferenceRow';
export interface RepositoryListItem {id:string;name:string;path?:string;pinned?:boolean;onOpen:()=>void;onPin?:()=>void;onForget?:()=>void;onClose?:()=>void}
export function RepositoryListSection({title,items,busy,empty}:{title:string;items:RepositoryListItem[];busy:boolean;empty:string}){
 const [message,setMessage]=useState('');
 const copy=async(path:string)=>{try{await navigator.clipboard.writeText(path);setMessage('Repository path copied.');}catch{setMessage('Could not copy the repository path.');}};
 return <TaskGroup id={`repositories:${title}`} title={title} count={items.length}><div className="repository-list-section">{items.map(item=><ReferenceRow key={item.id} label={item.name} description={item.path} title={item.path??item.name} icon={<GitBranch size={18}/>} disabled={busy} busy={busy} onSelect={item.onOpen} menuLabel={`Repository actions: ${item.name}`} actions={[{label:'Open repository',action:item.onOpen},...(item.onPin?[{label:item.pinned?'Remove from favorites':'Add to favorites',action:item.onPin}]:[]),...(item.path?[{label:'Copy path',action:()=>void copy(item.path!)}]:[]),...(item.onForget?[{label:'Remove from recent list',action:item.onForget}]:[]),...(item.onClose?[{label:'Close repository tab',action:item.onClose}]:[])]}/>)}{!items.length&&<p className="history-base">{empty}</p>}{message&&<p role="status">{message}</p>}</div></TaskGroup>;
}
