import {useInitialVisibleRead} from './useInitialVisibleRead';
import {WipTable} from './ui/WipTable';
import {useEffect,useRef,useState} from 'react';
import {api} from './api';
import type {NamedWorkspaceGroup} from './namedWorkspaces';
import type {WorkspaceWipListing} from './workspaceWips';
export function WorkspaceWips({group,query,filtered,disabled,onSelect,onOpenPath}:{group:NamedWorkspaceGroup;query:string;filtered:boolean;disabled:boolean;onSelect?:(id:string)=>void;onOpenPath?:(path:string)=>void}){
 const [listing,setListing]=useState<WorkspaceWipListing|null>(null),[busy,setBusy]=useState(false),[error,setError]=useState('');
 const [showClean,setShowClean]=useState(false);
 const generation=useRef(0),working=useRef(false);
 const scope=JSON.stringify([group.id,group.repositories]);
 useEffect(()=>{generation.current++;working.current=false;setListing(null);setBusy(false);setError('');return()=>{generation.current++;};},[scope]);
 const refresh=async()=>{if(disabled||working.current)return;const token=generation.current;working.current=true;setBusy(true);setError('');try{const result=await api.readWorkspaceWips(group.id);if(token===generation.current){if(result.workspaceId!==group.id)throw Error('Workspace response did not match.');setListing(result);}}catch(e){if(token===generation.current)setError(String(e));}finally{if(token===generation.current){working.current=false;setBusy(false);}}};
 useInitialVisibleRead(!filtered,disabled,refresh,scope);
 const terms=query.toLowerCase().split(/\s+/).filter(Boolean);
 const entries=(listing?.entries??[]).filter(e=>(showClean||e.state!=='clean'||!!e.warning)&&terms.every(t=>[e.repository,e.localPath,e.branch].filter(Boolean).join(' ').toLowerCase().includes(t)));
 return <section aria-label="Workspace WIPs"><h4>WIPs · Local changes</h4>
  <label><input type="checkbox" checked={showClean} onChange={event=>setShowClean(event.target.checked)}/>Show clean repositories</label>
  <button disabled={disabled||busy} onClick={()=>void refresh()}>Refresh WIPs</button>
  {busy&&<p role="status">Reading open repositories…</p>}
  {error&&<p role="alert">{error}{listing?' · Previous results retained; refresh required.':''}</p>}
  {!listing&&!busy&&<p>Only linked, already-open repositories can be checked. Use Refresh WIPs to retry a failed read.</p>}
  {filtered?<p>WIPs have no author, label or milestone metadata. Clear these filters to see local changes.</p>:<>
   {listing&&!entries.length&&<p>No matching WIPs. {listing.entries.filter(e=>e.state==='clean').length} repositories checked clean.</p>}
   <WipTable entries={entries} busy={disabled||busy} onSelect={onSelect} onOpenPath={onOpenPath} empty={busy?'Loading local changes…':listing?'No matching WIPs.':error?'Unable to load local changes. Use Refresh WIPs to retry.':'Waiting to read local changes.'}/>

  </>}
 </section>;
}
