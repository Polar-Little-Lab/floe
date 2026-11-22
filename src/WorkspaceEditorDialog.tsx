import {WorkspaceIconPicker,type WorkspaceIconName} from './ui/WorkspaceIcon';
import {workspaceColors} from './namedWorkspaces';
import {RepositoryScopePicker} from './ui/RepositoryScopePicker';
import type {RepositorySuggestion} from './launchpadRepositoryScope';
export function WorkspaceEditorDialog({editing,name,description,color,icon,onIcon,onDescription,onColor,repositories,suggestions,disabled,busy,error,notice,onName,onRepositories,onAdd,onSave,onClear,onClose}:{editing:boolean;name:string;icon:WorkspaceIconName;onIcon:(value:WorkspaceIconName)=>void;description:string;color:number;onDescription:(value:string)=>void;onColor:(value:number)=>void;repositories:string;suggestions:RepositorySuggestion[];disabled:boolean;busy:boolean;error:string;notice:string;onName:(value:string)=>void;onRepositories:(value:string)=>void;onAdd:(repository:string)=>void;onSave:()=>void;onClear:()=>void;onClose:()=>void}){
 const title=editing?'Edit Workspace':'Create Workspace';
 return <div className="modal-backdrop workspace-editor-backdrop"><section className="modal workspace-editor-dialog" role="dialog" aria-modal="true" aria-label={title}>
  <header className="modal-header"><h2>{title}</h2><button data-modal-close disabled={busy} onClick={onClose} aria-label="Close workspace editor">×</button></header>
  <form onSubmit={event=>{event.preventDefault();if(!disabled)onSave();}}>
   <div className="workspace-identity-row"><WorkspaceIconPicker value={icon} disabled={disabled} onChange={onIcon}/><label>Workspace Name *<input autoFocus className="wide-input" aria-label="Workspace name" required maxLength={256} value={name} disabled={disabled} onChange={event=>onName(event.target.value)}/></label>
   <label>Color<select aria-label="Workspace color" disabled={disabled} value={color} onChange={event=>onColor(Number(event.target.value))}>{workspaceColors.map((value,index)=><option key={value} value={index}>Graph color {index+1}</option>)}</select><span className="workspace-color-swatch" style={{backgroundColor:workspaceColors[color]}} aria-hidden="true"/></label></div>
   <label>Description<textarea className="wide-input" aria-label="Workspace description" placeholder="Describe the purpose of the workspace" rows={3} maxLength={4096} value={description} disabled={disabled} onChange={event=>onDescription(event.target.value)}/></label>
   <label>Repositories<textarea className="wide-input" aria-label="Workspace repositories" placeholder="owner/repository, one per line" rows={7} maxLength={20200} value={repositories} disabled={disabled} onChange={event=>onRepositories(event.target.value)}/></label>
   <RepositoryScopePicker suggestions={suggestions} label="Add current GitHub remote to workspace" placeholder="Add repository from current remotes…" disabled={disabled} onSelect={onAdd}/>
   <p className="history-base">Saved on this device. Closing keeps this draft until you leave Launchpad.</p>
   {notice&&<p role="status">{notice}</p>}{error&&<p role="alert">{error}</p>}
   <footer className="setting-row"><button type="button" disabled={busy} onClick={onClear}>Clear workspace draft</button><span className="grow"/><button type="button" disabled={busy} onClick={onClose}>Close</button><button className="primary" type="submit" disabled={disabled||!name.trim()}>{busy?'Saving…':editing?'Save Changes':'Create Workspace'}</button></footer>
  </form>
 </section></div>;
}
