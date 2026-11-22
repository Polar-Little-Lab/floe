import { GitBranch,X } from './OriginalIcons';
export interface RepositoryTab {repoId:string;name:string}
export function RepositoryTabs({tabs,active,busy,onSelect,onClose}:{tabs:RepositoryTab[];active:string;busy:boolean;onSelect:(id:string)=>void;onClose:(id:string)=>void}) {
 return <div className="repository-tabs" role="tablist" aria-label="Repository tabs">{tabs.map(tab=><div className={`repository-tab top-tab ${tab.repoId===active?'active':''}`} key={tab.repoId}><button type="button" role="tab" aria-selected={tab.repoId===active} disabled={busy} onClick={()=>onSelect(tab.repoId)}><GitBranch size={12}/><strong>{tab.name}</strong></button><button type="button" disabled={busy} aria-label={`Close repository ${tab.name}`} onClick={()=>onClose(tab.repoId)}><X size={14}/></button></div>)}</div>;
}
