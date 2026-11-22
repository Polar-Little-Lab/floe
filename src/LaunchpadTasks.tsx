import {LaunchpadSlots} from './ui/LaunchpadSlots';
import {LaunchpadSearchProvider,LaunchpadTaskToolbar} from './ui/LaunchpadSearch';
import {ProviderNavigation,useProviderNavigation} from './ProviderNavigation';
import {TaskGroupProvider} from './ui/TaskGroup';
import {ProviderTablePreferences} from './ui/ProviderTable';
import {LaunchpadViewSettings} from './LaunchpadViewSettings';
import {useLaunchpadViewPreferences} from './launchpadViewPreferences';
import {readGeneralPreferences} from './generalPreferences';
import {Users,Settings,Plus} from './OriginalIcons';
import {ScreenNavigation} from './ui/WorkspaceScreen';
import type {ReactNode} from 'react';
import {SavedTaskPreferences,type TaskPreferences} from './TaskPreferenceControls';
import type {TaskSnooze,TaskIdentity} from './taskSnoozes';
import {WorkspaceTaskResults} from './WorkspaceTaskResults';
import {api} from './api';
import type {NamedWorkspaceGroup,NamedWorkspaceState,SavedWorkspaceView} from './namedWorkspaces';
import {LaunchpadWorkspaces} from './LaunchpadWorkspaces';
import {useRef,useState} from 'react';
import {LaunchpadPullRequests} from './LaunchpadPullRequests';
import {LaunchpadIssues} from './LaunchpadIssues';
type TaskView='all'|'pulls'|'issues'|'wips'|'snoozed';
type LaunchpadTasksProps={disabled?:boolean;onSelect?:(id:string)=>void;onOpenPath?:(path:string)=>void;repositories?:ReactNode|((actions:{onNewWorkspace:()=>void;workspaceDisabled:boolean})=>ReactNode);openWips?:ReactNode};
export function LaunchpadTasks(props:LaunchpadTasksProps){const navigation=useProviderNavigation();return <LaunchpadSearchProvider>{navigation?<LaunchpadTasksContent {...props}/>:<ProviderNavigation><LaunchpadTasksContent {...props}/></ProviderNavigation>}</LaunchpadSearchProvider>;}
function LaunchpadTasksContent({disabled=false,onSelect,onOpenPath,repositories,openWips}:LaunchpadTasksProps){
 const navigation=useProviderNavigation();
 const [workspaceEditorRequest,setWorkspaceEditorRequest]=useState(0);
 const [group,setGroup]=useState<NamedWorkspaceGroup|null>(null);
 const [settings,setSettings]=useState<NamedWorkspaceState|null>(null),[saving,setSaving]=useState(false),[managementBusy,setManagementBusy]=useState(false);
 const savingRef=useRef(false);
 const saveViews=async(views:SavedWorkspaceView[])=>{if(disabled||managementBusy||savingRef.current||!settings?.workspace||settings.warning||!group)throw Error('Workspace settings are not ready.');savingRef.current=true;setSaving(true);try{const workspace={...settings.workspace,groups:settings.workspace.groups.map(g=>g.id===group.id?{...g,views}:g)};setSettings(await api.saveNamedWorkspaces(settings.revision,workspace));}finally{savingRef.current=false;setSaving(false);}};
 const saveSnoozes=async(snoozes:TaskSnooze[])=>{if(disabled||managementBusy||savingRef.current||!settings||settings.warning)throw Error('Workspace settings are not ready.');savingRef.current=true;setSaving(true);try{setSettings(await api.saveNamedWorkspaces(settings.revision,{...(settings.workspace??{groups:[],selected:null}),snoozes}));}finally{savingRef.current=false;setSaving(false);}};
 const savePins=async(pins:TaskIdentity[])=>{if(disabled||managementBusy||savingRef.current||!settings||settings.warning)throw Error('Workspace settings are not ready.');savingRef.current=true;setSaving(true);try{setSettings(await api.saveNamedWorkspaces(settings.revision,{...(settings.workspace??{groups:[],selected:null}),pins}));}finally{savingRef.current=false;setSaving(false);}};
 const reloadViews=async()=>{if(savingRef.current||managementBusy)throw Error('Workspace settings are busy.');savingRef.current=true;setSaving(true);try{setSettings(await api.loadNamedWorkspaces());}finally{savingRef.current=false;setSaving(false);}};
 const preferences:TaskPreferences={pins:settings?.workspace?.pins??[],snoozes:settings?.workspace?.snoozes??[],savePins,saveSnoozes};
 const [view,setView]=useState<TaskView|'repositories'>(()=>readGeneralPreferences().launchpadView);
 const changeView=(next:TaskView|'repositories',applied?:()=>void)=>{if(next===view&&!applied)return;navigation?.request(()=>{setView(next);applied?.();});};
 const [viewSelectionVersion,setViewSelectionVersion]=useState(0);
 const tablePreferences=useLaunchpadViewPreferences();
 const [viewSettings,setViewSettings]=useState(false);
 const [filterHost,setFilterHost]=useState<HTMLSpanElement|null>(null),[refreshHost,setRefreshHost]=useState<HTMLSpanElement|null>(null);
 const [saveViewActionHost,setSaveViewActionHost]=useState<HTMLSpanElement|null>(null);
 return <LaunchpadSlots.Provider value={{scope:view==='repositories'?null:group?'workspace':view==='pulls'?'pulls':view==='issues'?'issues':null,filters:filterHost,refresh:refreshHost}}><TaskGroupProvider><ProviderTablePreferences.Provider value={tablePreferences.value}><section aria-label="Launchpad tasks">
 <div className="launchpad-navigation-bar">
  <ScreenNavigation label="Task view" value={view} disabled={disabled||saving||managementBusy} onChange={next=>changeView(next,()=>setViewSelectionVersion(value=>value+1))} items={[{id:'pulls',label:'Pull requests'},{id:'issues',label:'Issues'},{id:'wips',label:'WIPs'},{id:'all',label:'All'},{id:'snoozed',label:'Snoozed'},...(repositories?[{id:'repositories' as const,label:'Repositories'}]:[])]}/>
  <span className="launchpad-save-view-slot" ref={setSaveViewActionHost} hidden={view==='repositories'}>{!group&&<button className="save-view-entry" disabled title="Select a workspace to save its task view"><Plus size={13}/>Save a View</button>}</span>
  <div className="launchpad-scope"><button aria-pressed="true" title="Tasks and workspaces on this device"><Users size={15}/>Personal</button><button disabled title="Team accounts and shared workspaces are not implemented">Team</button><button aria-expanded={viewSettings} onClick={()=>setViewSettings(!viewSettings)}><Settings size={15}/>View Settings</button></div></div>
 {viewSettings&&<LaunchpadViewSettings value={tablePreferences.value} onChange={tablePreferences.save} onReset={tablePreferences.reset} warning={tablePreferences.warning} onClose={()=>setViewSettings(false)}/>}
  <div hidden={view!=='repositories'}>{typeof repositories==='function'?repositories({workspaceDisabled:disabled||saving||managementBusy||!settings||!!settings.warning,onNewWorkspace:()=>{if(disabled||saving||managementBusy||!settings||settings.warning)return;navigation?.request(()=>{setView('all');setWorkspaceEditorRequest(value=>value+1);});}}):repositories}</div>
  <div hidden={view==='repositories'}>
  <LaunchpadTaskToolbar onRefreshHost={setRefreshHost}/>
  <div className="workspace-management">
   <LaunchpadWorkspaces editorRequest={workspaceEditorRequest} onFilterHost={setFilterHost} compact disabled={disabled||saving} onSelection={setGroup} externalState={settings} onState={setSettings} onBusy={setManagementBusy}/>
  </div>
  {managementBusy&&<p role="status">Loading or saving workspaces…</p>}
  {!managementBusy&&!settings&&<p role="alert">Workspace settings are unavailable. Select Reload workspaces to try again.</p>}
  {settings?.warning&&<p role="alert">{settings.warning}</p>}
  {group&&<WorkspaceTaskResults viewSelectionVersion={viewSelectionVersion} saveViewActionHost={saveViewActionHost} key={JSON.stringify([group.id,group.repositories.map(r=>[r.owner,r.repo])])} group={group} view={view==='repositories'?'all':view} onSelect={onSelect} onOpenPath={onOpenPath} snoozes={settings?.workspace?.snoozes??[]} onSaveSnoozes={saveSnoozes} pins={settings?.workspace?.pins??[]} onSavePins={savePins} disabled={disabled||saving||managementBusy} onViewChange={changeView} onSaveViews={saveViews} onReloadViews={reloadViews}/>}
  {!group&&(view==='wips'||view==='all')&&(openWips??<p>Open a repository to check local changes.</p>)}
  {!group&&view!=='wips'&&(view==='snoozed'||preferences.pins.length>0)&&<SavedTaskPreferences kind={view==='pulls'||view==='issues'?view:'all'} preferences={preferences} mode={view==='snoozed'?'snoozes':'pins'} disabled={disabled||saving||managementBusy||!settings||!!settings.warning}/>}
  <div hidden={!!group||view==='issues'||view==='wips'||view==='snoozed'}><LaunchpadPullRequests disabled={disabled||saving||managementBusy||!settings||!!settings.warning} preferences={preferences}/></div>
  <div hidden={!!group||view==='pulls'||view==='wips'||view==='snoozed'}><LaunchpadIssues disabled={disabled||saving||managementBusy||!settings||!!settings.warning} preferences={preferences}/></div>
  </div>
 </section></ProviderTablePreferences.Provider></TaskGroupProvider></LaunchpadSlots.Provider>;
}
