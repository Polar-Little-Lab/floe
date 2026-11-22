import {useState} from 'react';
import {referenceMatcher} from './referenceFilter';
import type {RemoteInfo} from './types';
import {ReferenceRow} from './ui/ReferenceRow';
import {ChevronDown,ChevronRight,GitBranch,X} from './OriginalIcons';
export function RemoteList({remotes,allowUnloaded=false,visibleOids=[],busy=false,filter='',onCreateBranch,onForgetCredential,onPushPreview,onRebase,onHead,onPrune,onTagPolicy,onPull,onFetch,onInspect,onSelect,onEdit,onRemove,onRename}:{remotes:RemoteInfo[];onCreateBranch?:(remote:string,branch:string,oid:string)=>void;allowUnloaded?:boolean;onForgetCredential?:(name:string,push:boolean,index:number)=>void;onPushPreview?:(name:string)=>void;onRebase?:(upstream:string)=>void;onHead?:(name:string)=>void;onPrune?:(name:string)=>void;onTagPolicy?:(name:string)=>void;visibleOids?:string[];busy?:boolean;filter?:string;onPull?:(name:string)=>void;onFetch?:(name:string)=>void;onInspect?:(name:string)=>void;onSelect?:(oid:string)=>void;onEdit?:(name:string)=>void;onRemove?:(name:string)=>void;onRename?:(name:string)=>void}) {

 const [collapsed,setCollapsed]=useState<string[]>([]),[details,setDetails]=useState<string|null>(null);
 const visible=new Set(visibleOids);const matches=referenceMatcher(filter);
 const branchMatches=(remote:RemoteInfo,branch:RemoteInfo['branches'][number])=>matches(`${remote.name}/${branch.name} ${branch.oid??''} ${branch.symbolicTarget??''}`);
 const filteredRemotes=remotes.filter(remote=>matches(remote.name)||remote.branches.some(branch=>branchMatches(remote,branch)));
 const detail=remotes.find(remote=>remote.name===details);
 return <div aria-label="Configured remotes">{remotes.length===0&&<p className="reference-empty">No configured remotes.</p>}{remotes.length>0&&!filteredRemotes.length&&<p className="reference-empty">No matching remote references.</p>}{filteredRemotes.map(remote=>{
  const branches=remote.branches.filter(b=>matches(remote.name)||branchMatches(remote,b));
  const expanded=!!filter.trim()||!collapsed.includes(remote.name);
  const action=(label:string,callback:((name:string)=>void)|undefined)=>({label,disabled:busy||!callback,action:()=>callback?.(remote.name)});
  return <section className="remote-tree-group" key={remote.name}>
   <ReferenceRow label={remote.name} icon={expanded?<ChevronDown size={13}/>:<ChevronRight size={13}/>} busy={busy} onSelect={()=>setCollapsed(expanded?[...collapsed,remote.name]:collapsed.filter(name=>name!==remote.name))} menuLabel={`Remote actions ${remote.name}`} actions={[
    action('Fetch',onFetch),action('Pull',onPull),action('Push…',onPushPreview),
    action('Edit remote…',onEdit),action('Rename remote…',onRename),action('Remove remote…',onRemove),
    action('Prune remote',onPrune),action('Set remote HEAD…',onHead),action('Fetch tag settings…',onTagPolicy),action('Check connection…',onInspect),
    {label:'Remote details…',disabled:busy,action:()=>setDetails(remote.name)}
   ]}/>
   {expanded&&<div className="remote-tree-branches">{branches.map(b=>b.symbolicTarget?<div className="remote-symbolic" key={b.name}>{b.name} → {b.symbolicTarget}</div>:<ReferenceRow key={b.name} label={b.name} icon={<GitBranch size={13}/>} accessibleLabel={`View remote branch ${remote.name}/${b.name}`} title={b.oid&&(allowUnloaded||visible.has(b.oid))?'View remote commit':'Commit is outside the loaded graph'} disabled={busy||!onSelect||!b.oid||(!allowUnloaded&&!visible.has(b.oid))} busy={busy} onSelect={()=>{if(b.oid)onSelect?.(b.oid);}} menuLabel={`Remote branch actions ${remote.name}/${b.name}`} actions={[
    {label:'Create local branch…',disabled:busy||!onCreateBranch||!b.oid||b.name==='HEAD',action:()=>{if(b.oid)onCreateBranch?.(remote.name,b.name,b.oid);}},
    {label:'Rebase current branch onto this…',disabled:busy||!onRebase||!b.oid||b.name==='HEAD',action:()=>onRebase?.(`remote:refs/remotes/${remote.name}/${b.name}`)}
   ]}/>)}</div>}
  </section>;
 })}
 {detail&&<div className="modal-backdrop"><section className="modal remote-details" role="dialog" aria-modal="true" aria-label={`Remote details ${detail.name}`}><header className="modal-header"><h2>{detail.name}</h2><button data-modal-close aria-label="Close remote details" disabled={busy} onClick={()=>setDetails(null)}><X size={18}/></button></header>
  {detail.fetchUrls.map((url,index)=><p key={'f'+index}>Fetch: {url}{onForgetCredential&&<button disabled={busy} aria-label={`Forget saved fetch login ${detail.name} URL ${index+1}`} onClick={()=>onForgetCredential(detail.name,false,index)}>Forget saved login</button>}</p>)}
  {(detail.pushUrls.length?detail.pushUrls:detail.fetchUrls).map((url,index)=><p key={'p'+index}>Push: {url}{!detail.pushUrls.length&&' (same as fetch)'}{onForgetCredential&&detail.pushUrls.length>0&&<button disabled={busy} aria-label={`Forget saved push login ${detail.name} URL ${index+1}`} onClick={()=>onForgetCredential(detail.name,true,index)}>Forget saved login</button>}</p>)}
  <p>Pull fetches the current upstream and opens merge or rebase preview according to repository settings. Push preview supports confirmed single-branch HTTPS transfers. On macOS, authentication is selected in the native confirmation.</p>
 </section></div>}
 </div>;
}
