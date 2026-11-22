import {ReferenceFolder} from './ui/ReferenceFolder';
import {useSidebarRead} from './useSidebarRead';
import {ReferenceGroupHeading} from './ui/ReferenceGroupHeading';
import {useEffect,useState} from 'react';
import {api,type GitflowSettings} from './api';
import type {Branch,Snapshot} from './types';
import type {GitflowKind} from './GitflowPanel';
import {ReferenceRow} from './ui/ReferenceRow';
import {GitBranch} from './OriginalIcons';

export function GitflowSidebar({snapshot,busy,demo,filter,run,onSelect,onCheckout,onOpen}:{snapshot:Snapshot;busy:boolean;demo:boolean;filter:string;run:(task:()=>Promise<void>)=>Promise<void>;onSelect:(oid:string)=>void;onCheckout:(name:string)=>void;onOpen:(kind?:GitflowKind,source?:string)=>void}){
 const [expanded,setExpanded]=useState(true);
 const {value:settings,error,reload}=useSidebarRead<GitflowSettings>(snapshot,expanded&&!demo,busy,api.gitflowSettings,run);
 const terms=filter.trim().toLocaleLowerCase().split(/\s+/).filter(Boolean);
 const matches=(branch:Branch)=>terms.every(term=>`${branch.name} ${branch.oid}`.toLocaleLowerCase().includes(term));
 useEffect(()=>{if(filter.trim())setExpanded(true);},[filter]);
 const writable=!busy&&!snapshot.writeBlock&&!snapshot.recoveryPending;
 const row=(branch:Branch,kind?:GitflowKind,prefix='')=><ReferenceRow key={`${kind??'base'}:${branch.name}`} label={branch.name.slice(prefix.length)} title={branch.name} current={branch.current} icon={<GitBranch size={13}/>} disabled={busy||!branch.oid} busy={busy} onSelect={()=>onSelect(branch.oid)} menuLabel={`Gitflow actions ${branch.name}`} actions={[{label:`Checkout ${branch.name}`,disabled:!writable||branch.current,action:()=>onCheckout(branch.name)},...(kind?[{label:`Finish ${branch.name}`,disabled:busy,action:()=>onOpen(kind,branch.name)}]:[])]}/>;
 const configured=settings&&!settings.warnings?.length&&!settings.missingBranches.length;
 return <section aria-label="Gitflow branches"><ReferenceGroupHeading label="GITFLOW" icon={<GitBranch size={16}/>} expanded={expanded} busy={busy} onToggle={()=>setExpanded(!expanded)} onOpen={()=>onOpen()}/>{expanded&&<>
 {demo?<button className="branch-item" onClick={()=>onOpen()}>Configure Gitflow</button>:error?<div className="reference-empty"><p role="alert">{error}</p><button disabled={busy} onClick={reload}>Reload Gitflow branches</button></div>:!settings?<p className="reference-empty" role="status">Loading Gitflow…</p>:!configured?<button className="branch-item" disabled={busy} onClick={()=>onOpen()}>Configure Gitflow</button>:<>
 {[settings.develop,settings.main].flatMap(name=>snapshot.branches.filter(branch=>branch.name===name&&matches(branch))).map(branch=>row(branch))}
 {(['feature','release','hotfix'] as const).map(kind=>{const prefix=settings[kind];if(prefix===null)return null;const allBranches=snapshot.branches.filter(branch=>branch.name!==settings.main&&branch.name!==settings.develop&&branch.name.startsWith(prefix)&&branch.name.length>prefix.length);const branches=allBranches.filter(matches);return allBranches.length?<ReferenceFolder key={kind+prefix} label={prefix.replace(/\/$/,'')||kind} count={branches.length} filtering={terms.length>0}>{branches.map(branch=>row(branch,kind,prefix))}</ReferenceFolder>:null;})}
 </>}
 </>}</section>;
}
