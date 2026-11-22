import {useEffect,useRef,useState} from 'react';
import {createPortal} from 'react-dom';
import {Plus,Settings,X} from './OriginalIcons';
import type {SavedWorkspaceView} from './namedWorkspaces';
type ViewState=Omit<SavedWorkspaceView,'id'|'name'>;
function sameView(a:ViewState,b:ViewState){return a.query===b.query&&a.kind===b.kind&&a.filters.author===b.filters.author&&a.filters.label===b.filters.label&&a.filters.milestone===b.filters.milestone;}
export function SavedWorkspaceViews({views,current,disabled,onSave,onApply,onReload,actionHost,selectionVersion=0}:{selectionVersion?:number;actionHost?:HTMLElement|null;views:SavedWorkspaceView[];current:ViewState;disabled:boolean;onSave:(views:SavedWorkspaceView[])=>Promise<void>;onApply:(view:SavedWorkspaceView,applied?:()=>void)=>void;onReload:()=>Promise<void>}){
 const [mode,setMode]=useState<'create'|'manage'|null>(null);
 const [selected,setSelected]=useState(''),[name,setName]=useState(''),[error,setError]=useState(''),[busy,setBusy]=useState(false),[removing,setRemoving]=useState(false);
 useEffect(()=>{setSelected('');},[selectionVersion]);
 const working=useRef(false),returnFocus=useRef<HTMLElement|null>(null);
 const selectedView=views.find(v=>v.id===selected),locked=disabled||busy;
 const active=selectedView&&sameView(selectedView,current)?selected:'';
 const close=()=>{if(working.current||disabled)return;setMode(null);setRemoving(false);requestAnimationFrame(()=>returnFocus.current?.isConnected&&returnFocus.current.focus());};
 const open=(next:'create'|'manage',target:HTMLElement)=>{if(locked)return;returnFocus.current=target;setName(next==='manage'?selectedView?.name??'':'');setError('');setRemoving(false);setMode(next);};
 const perform=async(action:()=>Promise<void>)=>{if(locked||working.current)return;working.current=true;setBusy(true);setError('');try{await action();}catch(e){setError(String(e).replace(/^Error: /,''));}finally{working.current=false;setBusy(false);}};
 const save=async()=>{
  const clean=name.trim();if(!clean||new TextEncoder().encode(clean).length>256||/[\u0000-\u001f\u007f]/.test(clean))throw Error('Enter a view name up to 256 bytes.');
  const update=mode==='manage';if(update&&!selectedView)throw Error('The selected view no longer exists.');if(!update&&views.length>=50)throw Error('Up to 50 saved views per workspace.');
  const value={...current,filters:{...current.filters},id:update?selected:crypto.randomUUID(),name:clean};
  await onSave(update?views.map(v=>v.id===selected?value:v):[...views,value]);setSelected(value.id);setMode(null);setRemoving(false);
  requestAnimationFrame(()=>returnFocus.current?.isConnected&&returnFocus.current.focus());
 };
 const apply=(view:SavedWorkspaceView)=>{if(locked||removing)return;onApply(view,()=>{setSelected(view.id);setError('');});};
 const actions=<><nav className="saved-view-tabs" aria-label="Saved workspace views">{views.map(view=><button key={view.id} title={view.name} disabled={locked} aria-pressed={active===view.id} aria-current={active===view.id?'page':undefined} onClick={()=>apply(view)}>{view.name}</button>)}</nav>{selectedView&&<button className="saved-view-manage" aria-label={`Manage saved view ${selectedView.name}`} title="Manage saved view" disabled={locked} onClick={event=>open('manage',event.currentTarget)}><Settings size={14}/></button>}<button className="save-view-entry" disabled={locked} onClick={event=>open('create',event.currentTarget)}><Plus size={13}/>Save a View</button></>;
 return <section className="saved-view-bar" aria-label="Saved workspace views controls">
  {actionHost?createPortal(actions,actionHost):<div className="saved-view-inline">{actions}</div>}
  {error&&!mode&&<p role="alert">{error}</p>}
  {mode&&<div className="modal-backdrop saved-view-backdrop"><section className="modal saved-view-dialog" role="dialog" aria-modal="true" aria-label={mode==='create'?'Save this Launchpad View':'Manage Launchpad View'} onKeyDown={event=>{if(event.key==='Escape'){event.preventDefault();event.stopPropagation();close();}}}>
   <header className="modal-header"><h2>{mode==='create'?'Save this Launchpad View':'Manage Launchpad View'}</h2><button type="button" data-modal-close disabled={locked} aria-label="Close saved view editor" onClick={close}><X size={18}/></button></header>
   <form onSubmit={event=>{event.preventDefault();if(!removing)void perform(save);}}>
    <input autoFocus className="wide-input" aria-label="Saved view name" placeholder="View name" value={name} maxLength={256} disabled={locked||removing} onChange={event=>setName(event.target.value)}/>
    {error&&<p role="alert">{error}</p>}
    {mode==='manage'&&<><p className="history-base">Update stores the current search, task type and filters in this view.</p><div className="setting-row"><button type="button" disabled={locked||removing||!selectedView} onClick={()=>setRemoving(true)}>Remove View</button><button type="button" disabled={locked||removing} onClick={()=>void perform(onReload)}>Reload saved views</button></div></>}
    {removing&&<div role="group" aria-label="Confirm saved view removal"><p>Remove saved view “{selectedView?.name??name}”?</p><button type="button" disabled={locked||!selectedView} onClick={()=>void perform(async()=>{await onSave(views.filter(view=>view.id!==selected));setSelected('');setMode(null);setRemoving(false);requestAnimationFrame(()=>returnFocus.current?.isConnected&&returnFocus.current.focus());})}>Confirm removal</button><button type="button" disabled={locked} onClick={()=>setRemoving(false)}>Keep View</button></div>}
    <footer><button type="button" disabled={locked} onClick={close}>Cancel</button><button className="primary" type="submit" disabled={locked||removing||!name.trim()||(mode==='manage'&&!selectedView)}>{busy?'Saving…':mode==='create'?'Create View':'Update View'}</button></footer>
   </form>
  </section></div>}
 </section>;
}
