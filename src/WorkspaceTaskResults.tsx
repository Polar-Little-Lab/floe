import {ProviderItemSummary,ProviderRepositorySummary,ProviderPeople} from './ui/ProviderItemSummary';
import {MetadataFilter} from './ui/MetadataFilter';
import {Tag,Users,FileText} from './OriginalIcons';
import {useTaskSearch} from './ui/LaunchpadSearch';
import {useProviderPanel} from './useProviderPanel';
import {TaskGroup} from './ui/TaskGroup';
import {ProviderTable} from './ui/ProviderTable';
import {ProviderFilterBar} from './ui/ProviderFilterBar';
import {ProviderItemPanel} from './ProviderItemPanel';
import {taskIdentity,taskIdentityKey,snoozeActive,snoozeTask,unsnoozeTask,type TaskSnooze,type TaskIdentity,setTaskPinned} from './taskSnoozes';
import {WorkspaceWips} from './WorkspaceWipResults';
import {SavedWorkspaceViews} from './SavedWorkspaceViews';
import {emptyTaskFilters,filterWorkspaceTasks,workspaceFilterOptions,type WorkspaceTaskFilters} from './workspaceTaskFilters';
import {useEffect,useRef,useState} from 'react';
import {api} from './api';
import {CopyTextButton} from './CopyTextButton';
import type {NamedWorkspaceGroup,SavedWorkspaceView} from './namedWorkspaces';
import {WorkspaceTaskFeed,type WorkspaceTaskState} from './workspaceTasks';
export function WorkspaceTaskResults({group,view,disabled=false,onViewChange,onSaveViews,onReloadViews,onSelect,onOpenPath,saveViewActionHost,viewSelectionVersion,snoozes,onSaveSnoozes,pins,onSavePins}:{viewSelectionVersion?:number;saveViewActionHost?:HTMLElement|null;pins:TaskIdentity[];onSavePins:(entries:TaskIdentity[])=>Promise<void>;snoozes:TaskSnooze[];onSaveSnoozes:(entries:TaskSnooze[])=>Promise<void>;onSelect?:(id:string)=>void;onOpenPath?:(path:string)=>void;group:NamedWorkspaceGroup;view:'all'|'pulls'|'issues'|'wips'|'snoozed';disabled?:boolean;onViewChange:(view:'all'|'pulls'|'issues'|'wips'|'snoozed',applied?:()=>void)=>void;onSaveViews:(views:SavedWorkspaceView[])=>Promise<void>;onReloadViews:()=>Promise<void>}){
 const [state,setState]=useState<WorkspaceTaskState>({sources:[],running:false,cancelled:false}),[query,setQuery]=useTaskSearch();
 const {panel,setPanel,requestPanel,registerNavigationGuard}=useProviderPanel<{repository:string;number:number;pull:boolean}>(previous=>{void feed.current?.refreshSource(previous.repository.toLowerCase()+':'+(previous.pull?'pulls':'issues'));});
 const [filters,setFilters]=useState<WorkspaceTaskFilters>(emptyTaskFilters);
 const scope=JSON.stringify([group.id,group.repositories.map(r=>[r.owner,r.repo])]);
 const feed=useRef<WorkspaceTaskFeed|null>(null);
 useEffect(()=>{const next=new WorkspaceTaskFeed(group,{pulls:api.readGithubPullRequests,issues:api.readGithubIssues},setState);feed.current=next;setState(next.state);return()=>{next.dispose();};},[scope]);
 const [now,setNow]=useState(Date.now),[snoozeError,setSnoozeError]=useState('');
 const snoozeBusy=useRef(false);
 useEffect(()=>{const timer=setInterval(()=>setNow(Date.now()),1000);return()=>clearInterval(timer);},[]);
 const active=new Set(snoozes.filter(e=>snoozeActive(e,now)).map(e=>taskIdentityKey(e.task)));
 const changeSnooze=async(repository:string,kind:'pulls'|'issues',id:string,hours:1|4|null)=>{if(disabled||snoozeBusy.current)return;snoozeBusy.current=true;setSnoozeError('');try{const task=taskIdentity(repository,kind,id);await onSaveSnoozes(hours===null?unsnoozeTask(snoozes,task):snoozeTask(snoozes,task,hours,Date.now()));setNow(Date.now());}catch(e){setSnoozeError(String(e));}finally{snoozeBusy.current=false;}};
 const pinned=new Set(pins.map(taskIdentityKey));
 const changePin=async(repository:string,kind:'pulls'|'issues',id:string)=>{if(disabled||snoozeBusy.current)return;snoozeBusy.current=true;setSnoozeError('');try{const task=taskIdentity(repository,kind,id);await onSavePins(setTaskPinned(pins,task,!pinned.has(taskIdentityKey(task))));}catch(e){setSnoozeError(String(e));}finally{snoozeBusy.current=false;}};
 const options=workspaceFilterOptions(state.sources);
 const memberships=new Set(group.repositories.map(r=>(r.owner+'/'+r.repo).toLowerCase()));
 const loadedKeys=new Set(state.sources.flatMap(source=>source.items.map(item=>taskIdentityKey(taskIdentity(source.repository,source.kind,item.id)))));
 const unloaded=snoozes.filter(e=>snoozeActive(e,now)&&memberships.has(e.task.repository)&&!loadedKeys.has(taskIdentityKey(e.task)));
 const unloadedPins=pins.filter(task=>memberships.has(task.repository)&&!loadedKeys.has(taskIdentityKey(task))&&active.has(taskIdentityKey(task))===(view==='snoozed')&&(view==='all'||view==='snoozed'||view===task.kind));
 const sources=state.sources.filter(s=>view==='all'||view==='snoozed'||s.kind===view);
 return <section aria-label="Workspace task results">
  {panel&&<ProviderItemPanel registerNavigationGuard={registerNavigationGuard} repository={panel.repository} number={panel.number} pull={panel.pull} onClose={refresh=>{const key=panel.repository.toLowerCase()+':'+(panel.pull?'pulls':'issues');setPanel(null);if(refresh)void feed.current?.refreshSource(key);}}/>}
  <SavedWorkspaceViews selectionVersion={viewSelectionVersion} actionHost={saveViewActionHost} views={group.views??[]} current={{query,kind:view,filters}} disabled={disabled} onSave={onSaveViews} onReload={onReloadViews} onApply={(saved,applied)=>onViewChange(saved.kind,()=>{setQuery(saved.query);setFilters({...saved.filters});applied?.();})}/>

  <ProviderFilterBar scope="workspace" canRefresh={group.repositories.length>0} loading={state.running} onStop={()=>feed.current?.cancel()} label="Workspace tasks" query={query} onQuery={setQuery} onRefresh={()=>void feed.current?.refresh()} onReset={()=>{setQuery('');setFilters({...emptyTaskFilters});}} active={!!query||Object.values(filters).some(value=>value!==null)} disabled={disabled||state.running} loaded={state.sources.some(source=>source.loaded)} summary={`${state.sources.filter(source=>source.loaded).length} of ${state.sources.length} sources loaded · filters cover loaded items only`}>
{(['author','label','milestone'] as const).map(kind=><MetadataFilter key={kind} label={`Filter workspace ${kind}`} allLabel={`All ${kind}s`} value={filters[kind]} options={options[kind]} onChange={value=>setFilters(previous=>({...previous,[kind]:value}))} missingLabel={kind==='milestone'?'Selected milestone':undefined} disabled={disabled||state.running||!state.sources.some(source=>source.loaded)} icon={kind==='label'?<Tag size={15}/>:kind==='author'?<Users size={15}/>:<FileText size={15}/>}/>)}
  </ProviderFilterBar>
  <details className="launchpad-storage-info"><summary>About pins and snoozes</summary><p className="history-base">Pins and snoozes are saved on this device only. Pinned items appear first within each loaded repository/task group. Expiry uses this device’s clock. Saved entries without loaded details are listed separately.</p></details>
  {snoozeError&&<p role="alert">{snoozeError} Reload saved views to refresh settings before retrying.</p>}
  {state.running&&<p role="status">Loading workspace sources…</p>}{state.cancelled&&<p>Loading stopped. Completed results are retained.</p>}
  {!group.repositories.length&&<p>This workspace has no repositories.</p>}
  {(view==='all'||view==='wips')&&<WorkspaceWips group={group} query={query} filtered={Object.values(filters).some(v=>v!==null)} disabled={disabled} onSelect={onSelect} onOpenPath={onOpenPath}/>}
  {view==='snoozed'&&unloaded.length>0&&<section aria-label="Snoozed tasks without loaded details"><h4>Saved snoozes · details not loaded</h4><p>These saved entries may be closed, deleted or on unloaded pages. Search and metadata filters do not apply to this list.</p>{unloaded.map(entry=><div className="recent-repository" key={taskIdentityKey(entry.task)}><div><strong>{entry.task.repository} · {entry.task.kind} · ID {entry.task.id}</strong><p>Until {new Date(entry.until).toLocaleString()}</p></div><button disabled={disabled} onClick={()=>void changeSnooze(entry.task.repository,entry.task.kind,entry.task.id,null)}>Unsnooze saved task</button></div>)}</section>}
  {unloadedPins.length>0&&<section aria-label="Pinned tasks without loaded details"><h4>Saved pins · details not loaded</h4><p>These saved entries may be closed, deleted or on unloaded pages. Search and metadata filters do not apply to this list.</p>{unloadedPins.map(task=><div className="recent-repository" key={taskIdentityKey(task)}><strong>{task.repository} · {task.kind} · ID {task.id}</strong><button disabled={disabled} onClick={()=>void changePin(task.repository,task.kind,task.id)}>Unpin saved task</button></div>)}</section>}
  {sources.map(source=>{const visible=filterWorkspaceTasks(source,query,filters).filter(item=>active.has(taskIdentityKey(taskIdentity(source.repository,source.kind,item.id)))===(view==='snoozed')).sort((a,b)=>Number(pinned.has(taskIdentityKey(taskIdentity(source.repository,source.kind,b.id))))-Number(pinned.has(taskIdentityKey(taskIdentity(source.repository,source.kind,a.id)))));return <TaskGroup key={source.key} id={`workspace:${group.id}:${source.key}`} title={`${source.repository} · ${source.kind==='pulls'?'Pull requests':'Issues'}`} count={visible.length} warning={!!source.error}>
   <p>{visible.length} of {source.items.length} loaded{source.nextPage!==null?' · more available':''}</p>
   {source.error&&<p role="alert">{source.error}{source.loaded?' · Previous results retained.':''}</p>}
   {source.loading&&<p role="status">Loading…</p>}
   {!source.loaded&&!source.loading&&!source.error&&<p>Not loaded.</p>}
   {source.loaded&&!source.items.length&&<p>{source.nextPage!==null?'No items in loaded pages. More pages are available.':'No open items.'}</p>}
   {source.items.length>0&&!visible.length&&<p>No matching loaded items.</p>}
   <ProviderTable label={`${source.repository} ${source.kind}`} empty={source.loading?'Loading…':source.loaded?'No matching loaded items.':'Load workspace tasks to see this repository.'} rows={visible.map(item=>({
    id:item.id,updatedAt:item.updatedAt,
    selected:panel?.number===item.number&&panel.repository.toLowerCase()===source.repository.toLowerCase()&&panel.pull===(source.kind==='pulls'),
    onOpen:disabled||state.running?undefined:()=>requestPanel({repository:source.repository,number:item.number,pull:source.kind==='pulls'}),
    status:'headBranch'in item?(item.draft===true?'Draft':item.draft===null?'Draft status unknown':'Open'):'Open',
    item:<ProviderItemSummary number={item.number} title={item.title} labels={item.labels} milestone={item.milestone} disabled={disabled||state.running} onOpen={()=>requestPanel({repository:source.repository,number:item.number,pull:source.kind==='pulls'})}/>,
    author:<ProviderPeople names={item.author?[item.author]:null}/>,
    collaborators:<ProviderPeople names={'headBranch'in item?item.requestedReviewers:item.assignees}/>,
    repository:<ProviderRepositorySummary repository={source.repository} head={'headBranch'in item?item.headBranch:undefined} base={'headBranch'in item?item.baseBranch:undefined}/>,
    actions:<><button disabled={disabled||state.running} onClick={()=>requestPanel({repository:source.repository,number:item.number,pull:source.kind==='pulls'})}>{source.kind==='pulls'?'Open Pull Request Panel':'Open Issue Panel'}</button><details className="provider-more-actions"><summary>More</summary><button disabled={disabled} aria-pressed={pinned.has(taskIdentityKey(taskIdentity(source.repository,source.kind,item.id)))} onClick={()=>void changePin(source.repository,source.kind,item.id)}>{pinned.has(taskIdentityKey(taskIdentity(source.repository,source.kind,item.id)))?'Unpin task':'Pin task'}</button>{view==='snoozed'?<button disabled={disabled} onClick={()=>void changeSnooze(source.repository,source.kind,item.id,null)}>Unsnooze</button>:<><button disabled={disabled} onClick={()=>void changeSnooze(source.repository,source.kind,item.id,1)}>Snooze 1 hour</button><button disabled={disabled} onClick={()=>void changeSnooze(source.repository,source.kind,item.id,4)}>Snooze 4 hours</button></>}<CopyTextButton text={item.url} label={`Copy ${source.repository} ${source.kind} #${item.number} URL`} disabled={disabled||state.running}/></details></>
   }))}/>

   {source.error&&<button disabled={disabled||state.running} onClick={()=>void feed.current?.loadSource(source.key)}>Retry {source.repository} {source.kind}</button>}
   {source.nextPage!==null&&!source.error&&<button disabled={disabled||state.running} onClick={()=>void feed.current?.loadSource(source.key,true)}>Load more {source.repository} {source.kind}</button>}
  </TaskGroup>})}
 </section>;
}
