import {api} from './api';
import {FilterSelect} from './ui/FilterSelect';
import {referenceMatcher} from './referenceFilter';
import {useRef,useState} from 'react';
import type {WorktreeListing,SubmoduleCheckout} from './api';
import type {Branch} from './types';
import {X,GitBranch} from './OriginalIcons';
import {ReferenceRow} from './ui/ReferenceRow';
export function WorktreesDialog({initialBranch,checkouts,onInspect,listing,branches,busy:parentBusy,canCreate,error,onClose,onReload,onCreate,onCopy,onTerminal,onLock,onOpen,onReveal,onPrune,onRemove}:{initialBranch?:string;checkouts:Record<string,SubmoduleCheckout>;onInspect:(name:string)=>void;onReveal?:(name:string)=>void;listing:WorktreeListing;branches:Branch[];busy:boolean;canCreate:boolean;error:string;onClose:()=>void;onReload:()=>void;onCreate:(name:string,path:string,branch:string|null)=>void;onCopy:(path:string)=>void;onTerminal:(name:string)=>void;onLock:(name:string,locked:boolean)=>void;onOpen:(path:string)=>void;onPrune?:(name:string,token:string)=>void;onRemove?:(name:string)=>void}) {
 const [picking,setPicking]=useState(false),[pathError,setPathError]=useState('');
 const choosing=useRef(false);
 const busy=parentBusy||picking;
 const [name,setName]=useState('');const [path,setPath]=useState('');const [branch,setBranch]=useState(initialBranch??'');
 const available=branches.filter(b=>!b.current&&b.oid&&!listing.entries.some(w=>w.branch===b.name));
 const [query,setQuery]=useState('');const [state,setState]=useState<'all'|'available'|'unavailable'|'locked'>('all');const [sort,setSort]=useState<'name'|'branch'|'path'>('name');
 const matches=referenceMatcher(query);
 const entries=listing.entries.filter(entry=>matches(`${entry.name} ${entry.branch} ${entry.path}`)&&(state==='all'||state==='available'&&entry.valid||state==='unavailable'&&!entry.valid||state==='locked'&&entry.locked)).sort((a,b)=>a[sort].localeCompare(b[sort])||a.name.localeCompare(b.name));
 const branchAvailable=!branch||available.some(value=>value.name===branch);
 const cleanName=name.trim();
 const nameError=!cleanName?'':! /^(?!-)[A-Za-z0-9_-]{1,80}$/.test(cleanName)?'Use up to 80 letters, numbers, hyphens or underscores; do not start with a hyphen.':listing.entries.some(entry=>entry.name===cleanName)?'A worktree with this name already exists.':!branch&&branches.some(entry=>entry.name===cleanName)?'A branch with this name exists. Select that branch or choose a different worktree name.':'';
 const browse=async()=>{if(busy||choosing.current||!cleanName||nameError)return;choosing.current=true;setPicking(true);setPathError('');try{const parent=await api.chooseWorktreeParent();if(parent){const separator=!parent.startsWith('/')&&parent.includes('\\')?'\\':'/';setPath(parent.replace(/[\\/]+$/,'')+separator+cleanName);}}catch(error){setPathError(String(error));}finally{choosing.current=false;setPicking(false);}};
 const ready=canCreate&&!busy&&branchAvailable&&!!cleanName&&!nameError&&!!path.trim();

 return <div className="modal-backdrop"><section className="modal worktrees-dialog" role="dialog" aria-modal="true" aria-label="Worktrees">
  <div className="modal-header"><h2>Worktrees</h2><button disabled={busy} data-modal-close aria-label="Close worktrees" onClick={onClose}><X size={18}/></button></div>
  <p>Linked worktrees share this repository's commits and branches. Open a linked worktree to browse history, stage or unstage files, hunks or selected lines, and commit. Other Git writes remain unavailable; repository policy may also require read-only mode.</p>
  <button disabled={busy} onClick={onReload}>Reload worktrees</button>
  <div className="worktree-list-tools"><input aria-label="Filter worktrees" placeholder="Filter name, branch or path…" maxLength={200} value={query} onChange={event=>setQuery(event.target.value)}/><select aria-label="Filter worktree state" value={state} onChange={event=>setState(event.target.value as typeof state)}><option value="all">All worktrees</option><option value="available">Available</option><option value="unavailable">Missing or invalid</option><option value="locked">Locked</option></select><select aria-label="Sort worktrees" value={sort} onChange={event=>setSort(event.target.value as typeof sort)}><option value="name">Name</option><option value="branch">Branch</option><option value="path">Path</option></select>{(query||state!=='all')&&<button onClick={()=>{setQuery('');setState('all');}}>Clear filters</button>}</div>
  <p>{entries.length} of {listing.entries.length} linked worktrees · {listing.entries.filter(entry=>entry.locked).length} locked · {listing.entries.filter(entry=>!entry.valid).length} missing or invalid</p>
  {!listing.entries.length&&<p>No linked worktrees.</p>}{!!listing.entries.length&&!entries.length&&<p>No matching worktrees. Creation below is independent of these filters.</p>}
  {entries.map(entry=><article className="worktree-entry" key={entry.name}><ReferenceRow label={entry.name} icon={<GitBranch size={16}/>} title={entry.path} disabled={busy||!entry.valid} busy={busy} onSelect={()=>onOpen(entry.path)} menuLabel={`Worktree actions: ${entry.name}`} actions={[
   {label:'Open this worktree',disabled:!entry.valid,action:()=>onOpen(entry.path)},
   ...(onRemove?[{label:'Remove this worktree',disabled:!canCreate||!entry.valid||entry.locked,action:()=>onRemove(entry.name)}]:[]),
   {label:checkouts[entry.name]?'Refresh checkout status':'Inspect checkout',disabled:!entry.valid,action:()=>onInspect(entry.name)},
   {label:'Open terminal',disabled:!entry.valid,action:()=>onTerminal(entry.name)},
   ...(onReveal?[{label:navigator.platform.includes('Mac')?'Open in Finder':'Reveal in Explorer',disabled:!entry.valid,action:()=>onReveal(entry.name)}]:[]),
   {label:'Copy path',action:()=>onCopy(entry.path)},
   {label:entry.locked?'Unlock this worktree':'Lock this worktree',disabled:!canCreate,action:()=>onLock(entry.name,entry.locked)},
   ...(onPrune?[{label:'Prune missing worktree',disabled:!canCreate||entry.valid||entry.locked||!entry.pruneToken,action:()=>{if(entry.pruneToken)onPrune(entry.name,entry.pruneToken);}}]:[]),
  ]}/><span>{entry.branch||'Unknown HEAD'} · {entry.valid?'Available':'Missing or invalid'}{entry.locked?' · Locked':''}</span><code>{entry.path}</code>{checkouts[entry.name]&&<section className="submodule-checkout" aria-label={`Checkout status for ${entry.name}`}><strong>{checkouts[entry.name].detached?'Detached HEAD':checkouts[entry.name].branch}</strong><code>{checkouts[entry.name].headOid??'No commit yet'}</code><span>{checkouts[entry.name].changed} changed paths · {checkouts[entry.name].staged} staged · {checkouts[entry.name].unstaged} unstaged/untracked</span><small>Last inspected snapshot. Refresh after external edits; staged and unstaged counts may overlap.</small>{checkouts[entry.name].writeBlock&&<p>{checkouts[entry.name].writeBlock}</p>}</section>}</article>)}
  {listing.truncated&&<p role="status">Only the first 200 linked worktrees are shown.</p>}
  <form onSubmit={event=>{event.preventDefault();if(ready)onCreate(cleanName,path.trim(),branch||null);}}>
   <h3>Create worktree</h3>{!branchAvailable&&<p role="alert">The selected branch "{branch}" is no longer available. Choose another branch before creating the worktree.</p>}
   <input autoFocus={!!initialBranch} className="wide-input" aria-label="Worktree name" aria-invalid={!!nameError} aria-describedby={nameError?'worktree-name-error':undefined} placeholder="Worktree name (letters, numbers, - or _)" maxLength={80} value={name} disabled={busy} onChange={e=>setName(e.target.value)}/>
   <input className="wide-input" aria-label="New worktree path" placeholder="Absolute path to a new folder outside this repository" maxLength={4096} value={path} disabled={busy} onChange={e=>setPath(e.target.value)}/>
   <button type="button" disabled={busy||!canCreate||!cleanName||!!nameError} onClick={()=>void browse()}>{picking?'Choosing folder…':'Choose parent folder…'}</button>{pathError&&<p role="alert">{pathError}</p>}
   {nameError&&<p id="worktree-name-error" role="alert">{nameError}</p>}
   <FilterSelect searchable label="Worktree branch" placeholder={branch&&!branchAvailable?`${branch} (unavailable)`:"Create new branch from HEAD (same name as worktree)"} value={branch} disabled={busy} options={[{value:'',label:'Create new branch from HEAD (same name as worktree)'},...available.map(value=>({value:value.name,label:value.name}))]} onChange={setBranch}/>

   <p>The parent folder must exist. Existing paths are not overwritten. Creation failures may leave a branch or partial folder for inspection.</p>
   <button disabled={!ready}>Create worktree</button>
  </form>
  {error&&<p role="alert">{error}</p>}
 </section></div>;
}
