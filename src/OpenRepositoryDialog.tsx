import {readGeneralPreferences} from './generalPreferences';
import { RecentRepositories } from './RecentRepositories';
import { useEffect,useRef,useState } from 'react';
import { Folder,X } from './OriginalIcons';
type Mode='open'|'create'|'clone';
type Props={path:string;busy:boolean;waitingLong?:boolean;error:string;onPath:(value:string)=>void;onClose:()=>void;onBrowse:()=>void;onOpen:()=>void;onCreate?:(branch:string)=>void;onClone?:(url:string,branch:string)=>void;initialMode?:Mode;recent?:string[];recentWarning?:string;onRecent?:(path:string)=>void;onForget?:(path:string)=>void};
export function OpenRepositoryDialog({path,busy,waitingLong=false,error,onPath,onClose,onBrowse,onOpen,onCreate,onClone,initialMode='open',recent=[],recentWarning,onRecent,onForget}:Props) {
 const [mode,setMode]=useState<Mode>(initialMode);
 const [branch,setBranch]=useState(()=>readGeneralPreferences().defaultBranch);
 const [cloneBranch,setCloneBranch]=useState('');
 const [url,setUrl]=useState('');
 const input=useRef<HTMLInputElement>(null);
 useEffect(()=>{if(!busy)input.current?.focus();},[busy]);
 const title=mode==='clone'?'Clone repository':mode==='create'?'Create repository':'Open repository';
 const disabled=busy||!path.trim()||(mode==='create'&&!branch.trim())||(mode==='clone'&&!url.trim());
 return <div className="modal-backdrop"><form className="modal" role="dialog" aria-modal="true" aria-label={title} onSubmit={e=>{e.preventDefault();if(disabled)return;if(mode==='create')onCreate?.(branch);else if(mode==='clone')onClone?.(url,cloneBranch);else onOpen();}}>
  <div className="modal-header"><h2>{title}</h2><button type="button" disabled={busy} aria-label="Close open repository" data-modal-close onClick={onClose}><X size={18}/></button></div>
  {(onCreate||onClone)&&<div className="segments"><button type="button" disabled={busy} aria-pressed={mode==='open'} onClick={()=>setMode('open')}>Open existing</button>{onCreate&&<button type="button" disabled={busy} aria-pressed={mode==='create'} onClick={()=>setMode('create')}>Create new</button>}{onClone&&<button type="button" disabled={busy} aria-pressed={mode==='clone'} onClick={()=>setMode('clone')}>Clone remote</button>}</div>}
  {mode==='clone'&&<><label>Repository URL<input className="wide-input" aria-label="Clone URL" autoComplete="off" autoCorrect="off" autoCapitalize="none" spellCheck={false} placeholder="https://host/owner/repository.git" value={url} maxLength={2048} disabled={busy} onChange={e=>setUrl(e.target.value)}/></label><p className="history-base">HTTPS repositories. Do not include passwords or tokens in this URL. On macOS, enter credentials in the native confirmation. SSH is not available yet.</p></>}
  <p>{mode==='open'?'Choose a local repository folder, or enter its full path.':'Enter a new folder path. Its parent folder must already exist.'}</p>
  <input ref={input} autoFocus className="wide-input" aria-label="Repository path" autoCorrect="off" autoCapitalize="none" spellCheck={false} placeholder="/Users/name/project or C:\\Users\\name\\project" value={path} maxLength={4096} disabled={busy} onChange={e=>onPath(e.target.value)}/>
  <p className="history-base">Use an absolute path. Quotes, ~ and environment variables are not expanded.</p>
  {mode==='create'&&<><input className="wide-input" aria-label="Initial branch" autoCorrect="off" autoCapitalize="none" spellCheck={false} value={branch} maxLength={200} disabled={busy} onChange={e=>setBranch(e.target.value)}/><p className="history-base">Existing paths are never overwritten. A failed creation may leave a folder for inspection.</p></>}
  {mode==='clone'&&<><input className="wide-input" aria-label="Clone branch (optional)" autoCorrect="off" autoCapitalize="none" spellCheck={false} placeholder="Default remote branch" value={cloneBranch} maxLength={200} disabled={busy} onChange={e=>setCloneBranch(e.target.value)}/><p className="history-base">Leave the branch blank to use the remote default. Existing paths are never overwritten. Worktrees requiring symlinks, submodules or attribute filters are not supported yet.</p></>}
  {busy&&waitingLong&&<p role="status" className="readonly-note">저장소 작업 응답을 기다리고 있습니다. 폴더 선택·확인 창이 열려 있는지 확인하세요. macOS에서는 파일 접근 확인이 다른 창에 표시될 수 있습니다. 아직 완료되거나 취소된 상태가 아닙니다.</p>}
  {error&&<p role="alert" className="readonly-note">{error}</p>}
  <div className="setting-row">{mode==='open'&&<button type="button" disabled={busy} onClick={onBrowse}><Folder size={16}/>Browse folders</button>}<button className="primary" disabled={disabled}>{busy?(mode==='clone'?'Cloning…':mode==='create'?'Creating…':'Opening…'):(mode==='clone'?'Clone repository':mode==='create'?'Create repository':'Open path')}</button></div>
  {recentWarning&&<p role="status" className="readonly-note">{recentWarning}</p>}
  {mode==='open'&&onRecent&&onForget&&<RecentRepositories paths={recent} busy={busy} onOpen={onRecent} onForget={onForget}/>}</form></div>;
}
