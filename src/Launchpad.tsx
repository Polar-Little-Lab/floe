import {TaskGroupProvider,TaskGroupControls} from './ui/TaskGroup';
import {ProviderNavigation,useProviderNavigation} from './ProviderNavigation';
import {LaunchpadRepositoryScope,type RepositorySuggestion} from './launchpadRepositoryScope';
import {api} from './api';
import type {OpenRepositoryPath} from './workspaceWips';
import {OpenRepositoryWips} from './OpenRepositoryWips';
import {WorkspaceScreen} from './ui/WorkspaceScreen';
import {LaunchpadTasks} from './LaunchpadTasks';
import {useEffect,useState} from 'react';
import {RepositoryListSection} from './ui/RepositoryListSection';
const PIN_KEY='branchdesk.repository-pins.v1';
function readPins():string[]{try{const value=JSON.parse(localStorage.getItem(PIN_KEY)??'[]');return Array.isArray(value)?[...new Set(value.filter((path):path is string=>typeof path==='string'&&path.length>0&&path.length<=32768&&!path.includes('\0')))].slice(0,50):[];}catch{return [];}}
import {Folder,GitBranch,Plus,Rocket} from './OriginalIcons';
import type {RepositoryTab} from './RepositoryTabs';
type LaunchpadProps={initialRepository?:string;suggestions?:RepositorySuggestion[];tabs:RepositoryTab[];recent:string[];busy:boolean;warning:string;error?:string;onClose:()=>void;onSelect:(id:string)=>void;onRecent:(path:string)=>void;onForget:(path:string)=>void;onCloseRepository?:(id:string)=>void;onStart:(mode:'open'|'create'|'clone')=>void};
export function Launchpad(props:LaunchpadProps){return <ProviderNavigation><LaunchpadContent {...props}/></ProviderNavigation>;}
function LaunchpadContent({suggestions=[],initialRepository,tabs,recent,busy,warning,error,onClose,onSelect,onRecent,onForget,onStart,onCloseRepository}:LaunchpadProps){
 const navigation=useProviderNavigation();
 const leave=(action:()=>void)=>{if(!busy)navigation?.request(action);};
 const selectRepository=(id:string)=>leave(()=>onSelect(id));
 const [query,setQuery]=useState('');
 const [openPaths,setOpenPaths]=useState<OpenRepositoryPath[]>([]),[pathWarning,setPathWarning]=useState('');
 const tabScope=JSON.stringify(tabs.map(tab=>tab.repoId));
 useEffect(()=>{let active=true;setOpenPaths([]);setPathWarning('');if(tabs.length)void api.openRepositoryPaths().then(paths=>{if(active)setOpenPaths(paths);}).catch(()=>{if(active)setPathWarning('Open repository paths could not be loaded. Names are still available.');});return()=>{active=false;};},[tabScope]);
 const pathFor=(id:string)=>openPaths.find(item=>item.repoId===id)?.path;

 const [pins,setPins]=useState<string[]>(readPins);const [pinWarning,setPinWarning]=useState('');const [alphabetical,setAlphabetical]=useState(false);
 useEffect(()=>{
  const synchronize=(event:StorageEvent)=>{
   if(event.storageArea!==localStorage||(event.key!==PIN_KEY&&event.key!==null))return;
   setPins(readPins());setPinWarning('');
  };
  window.addEventListener('storage',synchronize);
  return()=>window.removeEventListener('storage',synchronize);
 },[]);
 const savePins=(next:string[])=>{setPins(next);try{localStorage.setItem(PIN_KEY,JSON.stringify(next));setPinWarning('');}catch{setPinWarning('Favorites changed for this session, but could not be saved.');}};
 const togglePin=(path:string)=>{if(pins.includes(path)){savePins(pins.filter(value=>value!==path));return;}if(pins.length>=50){setPinWarning('Up to 50 repositories can be pinned. Unpin one to add another.');return;}savePins([...pins,path]);};
 const order=(paths:string[])=>alphabetical?[...paths].sort((a,b)=>(a.split(/[\\/]/).pop()??a).localeCompare(b.split(/[\\/]/).pop()??b)):paths;
 const row=(path:string,pinned:boolean)=>({id:path,name:path.replace(/[\\/]+$/,'').split(/[\\/]/).pop()||path,path,pinned,onOpen:()=>leave(()=>onRecent(path)),onPin:()=>togglePin(path),onForget:recent.includes(path)?()=>onForget(path):undefined});
 const terms=query.trim().toLocaleLowerCase().split(/\s+/).filter(Boolean);
 const matches=(value:string)=>terms.every(term=>value.toLocaleLowerCase().includes(term));
 const repositories=({onNewWorkspace,workspaceDisabled}:{onNewWorkspace:()=>void;workspaceDisabled:boolean})=><TaskGroupProvider>
  <div className="launchpad-repository-actions"><button disabled={busy} onClick={()=>leave(()=>onStart('open'))}><Folder size={16}/>Browse</button><button disabled={busy} onClick={()=>leave(()=>onStart('clone'))}><GitBranch size={16}/>Clone</button><button disabled={busy} onClick={()=>leave(()=>onStart('create'))}><Plus size={16}/>Init</button><button disabled={workspaceDisabled} onClick={onNewWorkspace}><Plus size={16}/>New Workspace</button></div>
  <div className="setting-row"><TaskGroupControls/><input className="wide-input" aria-label="Filter repositories" placeholder="Search repository name or path" maxLength={4096} value={query} onChange={e=>setQuery(e.target.value)}/>{query&&<button onClick={()=>setQuery('')}>Clear search</button>}</div>
  <RepositoryListSection title="Open repositories" busy={busy} empty={tabs.length?'No open repositories match your search.':'No repositories are open.'} items={tabs.filter(t=>matches(t.name+' '+(pathFor(t.repoId)??''))).sort((a,b)=>alphabetical?a.name.localeCompare(b.name):0).map(t=>{const path=pathFor(t.repoId);return {id:t.repoId,name:t.name,path,pinned:path?pins.includes(path):false,onOpen:()=>selectRepository(t.repoId),onPin:path?()=>togglePin(path):undefined,onClose:onCloseRepository?()=>leave(()=>onCloseRepository(t.repoId)):undefined};})}/>
  {pathWarning&&<p role="status">{pathWarning}</p>}
  <div className="launchpad-list-heading"><h2><GitBranch size={22}/>Saved repositories</h2><button aria-pressed={alphabetical} onClick={()=>setAlphabetical(!alphabetical)}>{alphabetical?'Sort: name':'Sort: recent'}</button></div>
  <RepositoryListSection title="Favorites" busy={busy} empty={pins.length?'No favorites match your search.':'Add repositories to favorites using their ⋮ menu.'} items={order(pins.filter(matches)).map(path=>row(path,true))}/>
  <RepositoryListSection title="Recent repositories" busy={busy} empty="No matching recent repositories." items={order(recent.filter(path=>!pins.includes(path)&&matches(path))).map(path=>row(path,false))}/>
  {pinWarning&&<p role="status">{pinWarning}</p>}
  {warning&&<p role="status">{warning}</p>}
  {error&&<p role="alert">{error}</p>}
 </TaskGroupProvider>;
 return <WorkspaceScreen modal={false} label="Launchpad" title="Launchpad" icon={<Rocket size={26}/>} busy={busy} onClose={()=>leave(onClose)} className="launchpad-screen">
  <LaunchpadRepositoryScope suggestions={suggestions} initialRepository={initialRepository}><LaunchpadTasks disabled={busy} onSelect={selectRepository} onOpenPath={path=>leave(()=>onRecent(path))} repositories={repositories} openWips={<OpenRepositoryWips tabs={tabs} disabled={busy} onSelect={selectRepository}/>}/></LaunchpadRepositoryScope>
 </WorkspaceScreen>;
}
