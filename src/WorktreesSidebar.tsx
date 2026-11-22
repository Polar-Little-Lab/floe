import {useSidebarRead} from './useSidebarRead';
import {useState} from 'react';
import {api,type WorktreeListing} from './api';
import type {Snapshot} from './types';
import {TreePine} from './OriginalIcons';
import {ReferenceRow} from './ui/ReferenceRow';
import {ReferenceGroupHeading} from './ui/ReferenceGroupHeading';
import {referenceMatcher} from './referenceFilter';
export function WorktreesSidebar({snapshot,busy,demo,filter,run,onManage,onOpen,onTerminal,onReveal}:{snapshot:Snapshot;busy:boolean;demo:boolean;filter:string;run:(task:()=>Promise<void>)=>Promise<void>;onManage:()=>void;onOpen:(path:string)=>void;onTerminal:(snapshot:Snapshot,name:string)=>void;onReveal:(snapshot:Snapshot,name:string)=>void}){
 const [expanded,setExpanded]=useState(false);
 const {value:listing,error,reload}=useSidebarRead<WorktreeListing>(snapshot,expanded&&!demo,busy,api.worktrees,run);
 const matches=referenceMatcher(filter);
 const entries=listing?.entries.filter(entry=>matches(`${entry.name} ${entry.branch} ${entry.path}`))??[];
 return <section aria-label="Linked worktrees"><ReferenceGroupHeading label="WORKTREES" icon={<TreePine size={16}/>} expanded={expanded} count={listing?.entries.length} busy={busy||demo} onToggle={()=>setExpanded(!expanded)} onOpen={onManage}/>{expanded&&<>
 {demo?<p className="reference-empty">Open a repository to view worktrees.</p>:error?<div className="reference-empty"><p role="alert">{error}</p><button disabled={busy} onClick={reload}>Reload worktrees</button></div>:!listing?<p className="reference-empty" role="status">Loading worktrees…</p>:<>
 {entries.map(entry=><ReferenceRow key={entry.name} label={entry.branch||entry.name} description={`${entry.name}${entry.locked?' · Locked':''}${entry.valid?'':' · Missing or invalid'}`} icon={<TreePine size={13}/>} title={entry.path} busy={busy} disabled={busy||!entry.valid} onSelect={()=>onOpen(entry.path)} menuLabel={`Worktree actions ${entry.name}`} actions={[
 {label:'Open this worktree',disabled:!entry.valid,action:()=>onOpen(entry.path)},
 {label:'Open terminal',disabled:!entry.valid,action:()=>onTerminal(snapshot,entry.name)},
 {label:navigator.platform.includes('Mac')?'Open in Finder':'Reveal in Explorer',disabled:!entry.valid,action:()=>onReveal(snapshot,entry.name)},
 {label:'Manage worktrees…',action:onManage},
 ]}/>)}
 {!entries.length&&<p className="reference-empty">{filter.trim()?'No matching worktrees.':'No linked worktrees.'}</p>}
 {listing.truncated&&<p className="reference-empty">First 200 worktrees shown.</p>}
 <button className="branch-item" disabled={busy} onClick={onManage}>Manage worktrees…</button>
 </>}
 </>}</section>;
}
