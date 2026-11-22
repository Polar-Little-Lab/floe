import {Folder,GitBranch,Rocket,Tag,Plus} from '../OriginalIcons';
export type WorkspaceIconName=''|'folder'|'branch'|'rocket'|'tag';
export function WorkspaceIcon({name,size=18}:{name?:WorkspaceIconName;size?:number}){
 const Icon=name==='folder'?Folder:name==='branch'?GitBranch:name==='rocket'?Rocket:name==='tag'?Tag:Plus;
 return <Icon size={size}/>;
}
export function WorkspaceIconPicker({value,disabled,onChange}:{value:WorkspaceIconName;disabled:boolean;onChange:(value:WorkspaceIconName)=>void}){
 return <details className="workspace-icon-picker"><summary aria-label="Choose workspace icon"><small>Icon</small><WorkspaceIcon name={value}/></summary><div role="group" aria-label="Workspace icons">{(['','folder','branch','rocket','tag'] as const).map(name=><button key={name} type="button" disabled={disabled} aria-label={name||'No workspace icon'} aria-pressed={value===name} onClick={event=>{onChange(name);event.currentTarget.closest('details')?.removeAttribute('open');}}><WorkspaceIcon name={name}/></button>)}</div></details>;
}
