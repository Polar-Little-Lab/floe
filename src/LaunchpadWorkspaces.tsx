import {WorkspaceLocalLinks} from './WorkspaceLocalLinks';
import {FilterSelect} from './ui/FilterSelect';
import {Settings,Folder} from './OriginalIcons';
import {useProviderNavigation} from './ProviderNavigation';
import {WorkspaceIcon,type WorkspaceIconName} from './ui/WorkspaceIcon';
import {WorkspaceEditorDialog} from './WorkspaceEditorDialog';
import {useLaunchpadRepositoryScope} from './launchpadRepositoryScope';
import {useEffect,useRef,useState} from 'react';
import {api} from './api';
import {workspaceAppearance,workspaceColors,workspaceDraft,type NamedWorkspace,type NamedWorkspaceState,type NamedWorkspaceGroup} from './namedWorkspaces';
export function LaunchpadWorkspaces({editorRequest=0,onFilterHost,compact=false,disabled=false,onSelection,externalState,onState,onBusy}:{editorRequest?:number;onFilterHost?:(element:HTMLSpanElement|null)=>void;compact?:boolean;disabled?:boolean;onSelection?:(group:NamedWorkspaceGroup|null)=>void;externalState?:NamedWorkspaceState|null;onState?:(state:NamedWorkspaceState|null)=>void;onBusy?:(busy:boolean)=>void}){
 const navigation=useProviderNavigation();
 const [manage,setManage]=useState(false);
 const [editorOpen,setEditorOpen]=useState(false);
 const handledEditorRequest=useRef(editorRequest);
 useEffect(()=>{if(editorRequest!==handledEditorRequest.current){handledEditorRequest.current=editorRequest;setEditorOpen(true);}},[editorRequest]);
 const [icon,setIcon]=useState<WorkspaceIconName>('');
 const [description,setDescription]=useState(''),[color,setColor]=useState(0);
 const {suggestions}=useLaunchpadRepositoryScope();
 const [repositoryNotice,setRepositoryNotice]=useState('');
 const [state,setState]=useState<NamedWorkspaceState|null>(null),[busy,setBusy]=useState(false),[error,setError]=useState('');
 const [editing,setEditing]=useState<string|null>(null),[name,setName]=useState(''),[repositories,setRepositories]=useState('');
 const [removing,setRemoving]=useState<string|null>(null);
 useEffect(()=>{if(externalState)setState(externalState);},[externalState]);
 useEffect(()=>{onState?.(state);},[state,onState]);
 useEffect(()=>{onBusy?.(busy);},[busy,onBusy]);
 const alive=useRef(true),working=useRef(false);
 const guarded=(action:()=>void)=>{if(disabled||working.current)return;if(navigation)navigation.request(action);else action();};
 const load=async()=>{if(working.current)return;working.current=true;setBusy(true);setError('');try{const loaded=await api.loadNamedWorkspaces();if(alive.current)setState(loaded);}catch(e){if(alive.current)setError('Could not load workspaces. Use the desktop app and reload to try again.');}finally{working.current=false;if(alive.current)setBusy(false);}};
 useEffect(()=>{alive.current=true;void load();return()=>{alive.current=false;};},[]);
 useEffect(()=>{setRemoving(null);},[state]);
 useEffect(()=>{const w=state?.workspace;onSelection?.(!state?.warning?w?.groups.find(g=>g.id===w.selected)??null:null);},[state,onSelection]);
 const current=state?.workspace??{groups:[],selected:null},locked=disabled||busy||!state||!!state.warning;
 const save=async(next:NamedWorkspace)=>{if(locked||working.current||!state)return false;working.current=true;setBusy(true);setError('');try{const result=await api.saveNamedWorkspaces(state.revision,next);if(!alive.current)return false;setState(result);return true;}catch(e){if(alive.current)setError(String(e));return false;}finally{working.current=false;if(alive.current)setBusy(false);}};

 const selectedIndex=current.groups.findIndex(g=>g.id===current.selected);
 const move=async(delta:number)=>{const target=selectedIndex+delta;if(selectedIndex<0||target<0||target>=current.groups.length)return;const groups=[...current.groups];[groups[selectedIndex],groups[target]]=[groups[target],groups[selectedIndex]];await save({...current,groups});};
 const remove=async()=>{if(!removing)return;const id=removing;if(await save({...current,groups:current.groups.filter(g=>g.id!==id),selected:current.selected===id?null:current.selected})){setRemoving(null);if(editing===id)reset();}};
 const edit=()=>{const group=current.groups.find(g=>g.id===current.selected);if(!group)return;setEditing(group.id);setName(group.name);setIcon(group.icon??'');setDescription(group.description??'');setColor(group.color??0);setRepositories(group.repositories.map(r=>r.owner+'/'+r.repo).join('\n'));setRepositoryNotice('');setManage(true);setEditorOpen(true);setError('');};
 const reset=()=>{setEditing(null);setName('');setIcon('');setDescription('');setColor(0);setRepositories('');setRepositoryNotice('');};
 const addRepository=(repository:string)=>{
  if(locked||removing)return;
  const lines=repositories.split(/\r?\n/).map(line=>line.trim()).filter(Boolean);
  if(lines.some(line=>line.toLowerCase()===repository.toLowerCase())){setRepositoryNotice('This repository is already in the draft.');return;}
  if(lines.length>=100){setRepositoryNotice('A workspace supports up to 100 repositories.');return;}
  setRepositories([...lines,repository].join('\n'));setRepositoryNotice('Repository added to the draft. Save the workspace to apply it.');
 };
 const submit=async()=>{try{const original=current.groups.find(g=>g.id===editing);if(editing&&!original)throw Error('This workspace was removed. Start a new workspace to save these values.');if(!editing&&current.groups.length>=50)throw Error('Up to 50 workspaces are supported.');const values=workspaceDraft(name,repositories,original?.repositories);const group={...original,id:editing??crypto.randomUUID(),...values,...workspaceAppearance(description,color,icon)};const next={...current,groups:editing?current.groups.map(g=>g.id===editing?group:g):[...current.groups,group],selected:group.id};if(await save(next)){reset();setEditorOpen(false);}}catch(e){setError(String(e).replace(/^Error: /,''));}};
 return <section aria-label="Named workspaces">
  {!compact&&<h3>Workspaces</h3>}{(!compact||manage)&&<p className="history-base">Organize repositories into a workspace. Workspaces are saved on this device.</p>}
  {busy&&<p role="status">Loading or saving workspaces…</p>}{error&&<p role="alert">{error}</p>}{state?.warning&&<p role="alert">{state.warning}</p>}
  <div className="setting-row">
   <FilterSelect label="Selected workspace" placeholder="Workspace" disabled={locked||!!removing} value={current.selected??''} options={[{value:'',label:'None'},...current.groups.map(g=>({value:g.id,label:g.name,icon:g.icon?<WorkspaceIcon name={g.icon}/>:<Folder size={16}/>}))]} onChange={value=>{const selected=value||null;if(selected!==current.selected)guarded(()=>{void save({...current,selected});});}}/>
   {compact&&<span className="launchpad-metadata-slot" ref={onFilterHost}/>}
   {compact&&<button aria-label={manage?'Close workspace settings':'Manage workspaces'} title="Manage workspaces" aria-expanded={manage} onClick={()=>setManage(!manage)}><Settings size={16}/></button>}{compact&&!state&&<button disabled={disabled||busy} onClick={()=>guarded(()=>{void load();})}>Reload workspaces</button>}
   <button hidden={compact&&!manage} disabled={locked||!!removing} onClick={()=>guarded(()=>setEditorOpen(true))}>{editing||name||repositories||description||color!==0||!!icon?'Continue workspace draft':'Create workspace'}</button>
   <span className="workspace-management-actions" hidden={compact&&!manage}><button disabled={locked||!!removing||!current.selected||!!editing||!!name||!!repositories||!!description||color!==0||!!icon} title="Finish or clear an existing draft before editing another workspace" onClick={()=>guarded(edit)}>Edit workspace</button>
   <button disabled={disabled||busy} onClick={()=>guarded(()=>{void load();})}>Reload workspaces</button>
   <button disabled={locked||!!removing||selectedIndex<=0} onClick={()=>guarded(()=>{void move(-1);})}>Move workspace up</button>
   <button disabled={locked||!!removing||selectedIndex<0||selectedIndex>=current.groups.length-1} onClick={()=>guarded(()=>{void move(1);})}>Move workspace down</button>
   <button disabled={locked||!!removing||!current.selected} onClick={()=>setRemoving(current.selected)}>Remove workspace</button></span>
  </div>
  {selectedIndex>=0&&(!compact||manage)&&<div className="workspace-selected-summary"><span className="workspace-color-swatch" style={{backgroundColor:workspaceColors[current.groups[selectedIndex].color??0]}} aria-hidden="true"/><WorkspaceIcon name={current.groups[selectedIndex].icon}/><strong>{current.groups[selectedIndex].name}</strong>{current.groups[selectedIndex].description&&<p>{current.groups[selectedIndex].description}</p>}</div>}
  <div hidden={compact&&!manage}>
  {selectedIndex>=0&&(!compact||manage)&&<WorkspaceLocalLinks key={current.selected} group={current.groups[selectedIndex]} disabled={locked||!!removing} onLink={(repository,path)=>{const groupId=current.selected;void save({...current,groups:current.groups.map(group=>group.id===groupId?{...group,repositories:group.repositories.map(item=>item.owner===repository.owner&&item.repo===repository.repo?{...item,localPath:path}:item)}:group)});}}/>}
  {removing&&<div role="group" aria-label="Confirm workspace removal"><p>Remove workspace “{current.groups.find(g=>g.id===removing)?.name}”? Only this saved group is removed. Repository files and remote repositories remain.</p><button disabled={locked} onClick={()=>guarded(()=>{void remove();})}>Confirm workspace removal</button><button disabled={busy} onClick={()=>setRemoving(null)}>Cancel workspace removal</button></div>}
 </div>
 {editorOpen&&<WorkspaceEditorDialog icon={icon} onIcon={setIcon} description={description} color={color} onDescription={setDescription} onColor={setColor} editing={!!editing} name={name} repositories={repositories} suggestions={suggestions} disabled={locked||!!removing} busy={busy||disabled} error={error||state?.warning||''} notice={repositoryNotice} onName={setName} onRepositories={setRepositories} onAdd={addRepository} onSave={()=>guarded(()=>{void submit();})} onClear={()=>{reset();setError('');}} onClose={()=>{if(!busy&&!working.current)setEditorOpen(false);}}/>}
 </section>;
}
