import {useInitialVisibleRead} from './useInitialVisibleRead';
import {useTaskSearch,useLaunchpadSearch} from './ui/LaunchpadSearch';
import {useEffect,useRef,useState} from 'react';
import {api} from './api';
import type {RepositoryTab} from './RepositoryTabs';
import type {WorkspaceWipEntry} from './workspaceWips';
import {WipTable} from './ui/WipTable';
export function OpenRepositoryWips({tabs,disabled,onSelect}:{tabs:RepositoryTab[];disabled:boolean;onSelect:(id:string)=>void}){
 const sharedSearch=useLaunchpadSearch();
 const [entries,setEntries]=useState<WorkspaceWipEntry[]>([]),[busy,setBusy]=useState(false),[loaded,setLoaded]=useState(false);
 const [query,setQuery]=useTaskSearch(),[showClean,setShowClean]=useState(false);
 const generation=useRef(0),working=useRef(false);
 const scope=JSON.stringify(tabs.map(tab=>[tab.repoId,tab.name]));
 const refresh=async()=>{
  if(disabled||working.current)return;const token=generation.current;working.current=true;setBusy(true);
  const next:WorkspaceWipEntry[]=[];
  // Native reads use a global operation lease; do not fan them out concurrently.
  for(const tab of tabs){
   if(token!==generation.current)return;
   try{const snapshot=await api.refresh(tab.repoId);if(snapshot.repoId!==tab.repoId)throw Error('Repository response did not match.');next.push({repository:tab.name,localPath:null,repoId:tab.repoId,state:snapshot.changes.length?'changes':'clean',branch:snapshot.detached?`Detached ${snapshot.headOid?.slice(0,7)??'HEAD'}`:snapshot.branch,changed:snapshot.changes.length,staged:snapshot.changes.filter(change=>change.staged).length,unstaged:snapshot.changes.filter(change=>change.unstaged).length,warning:snapshot.writeBlock??(snapshot.recoveryPending?'Recovery pending':null)});}
   catch(error){next.push({repository:tab.name,localPath:null,repoId:tab.repoId,state:'error',branch:null,changed:null,staged:null,unstaged:null,warning:String(error)});}
  }
  if(token!==generation.current)return;setEntries(next);setLoaded(true);working.current=false;setBusy(false);
 };
 useEffect(()=>{generation.current++;working.current=false;setEntries([]);setLoaded(false);setBusy(false);return()=>{generation.current++;};},[scope]);
 useInitialVisibleRead(true,disabled,refresh,scope);
 const terms=query.toLocaleLowerCase().split(/\s+/).filter(Boolean);
 const visible=entries.filter(entry=>(showClean||entry.state!=='clean'||!!entry.warning)&&terms.every(term=>[entry.repository,entry.branch].join(' ').toLocaleLowerCase().includes(term)));
 return <section aria-label="Open repository WIPs"><h3>WIPs · Open repositories</h3><div className="setting-row">{!sharedSearch&&<input aria-label="Filter open repository WIPs" placeholder="Search repository or branch" value={query} maxLength={4096} onChange={event=>setQuery(event.target.value)}/>}<label><input type="checkbox" checked={showClean} onChange={event=>setShowClean(event.target.checked)}/>Show clean repositories</label><button disabled={disabled||busy} onClick={()=>void refresh()}>Refresh WIPs</button></div>{busy&&<p role="status">Reading open repositories…</p>}{loaded&&<p className="history-base">{entries.length} checked · {entries.filter(entry=>entry.state==='changes').length} with changes · {entries.filter(entry=>entry.state==='error').length} could not be read</p>}<WipTable entries={visible} busy={disabled||busy} onSelect={onSelect} empty={!tabs.length?'Open a repository to see work in progress.':busy?'Loading local changes…':loaded?'No matching WIPs.':'Local changes have not been loaded.'}/></section>;
}
