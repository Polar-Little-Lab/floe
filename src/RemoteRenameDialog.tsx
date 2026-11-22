import {useState} from 'react';
import {X} from './OriginalIcons';
export function RemoteRenameDialog({name,busy,error,onClose,onRename}:{name:string;busy:boolean;error:string;onClose:()=>void;onRename:(value:string)=>void}){
 const [value,setValue]=useState(name);
 const valid=!!value.trim()&&value.trim()!==name;
 return <div className="modal-backdrop"><form className="modal" role="dialog" aria-modal="true" aria-label="Rename remote" onSubmit={e=>{e.preventDefault();if(!busy&&valid)onRename(value.trim());}}><div className="modal-header"><h2>Rename remote {name}</h2><button type="button" disabled={busy} data-modal-close onClick={onClose} aria-label="Close remote rename"><X size={18}/></button></div><label>New remote name<input autoFocus className="wide-input" aria-label="New remote name" value={value} disabled={busy} maxLength={200} spellCheck={false} onChange={e=>setValue(e.target.value)}/></label><p>Updates local tracking references, default fetch rules, and branch upstream and push settings. Custom fetch destinations retain their existing paths.</p><p>The server repository and local branches stay unchanged.</p>{error&&<p role="alert" className="readonly-note">{error}</p>}<button className="primary" disabled={busy||!valid}>{busy?'Renaming…':'Rename remote'}</button></form></div>;
}
