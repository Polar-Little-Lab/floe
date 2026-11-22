import {ChangeStats,type ChangeStatistics} from './ui/ChangeStats';
import {useInitialVisibleRead} from './useInitialVisibleRead';
import {DiffFileNavigator} from './DiffFileNavigator';
import {CopyTextButton} from './CopyTextButton';
import {useRef,useState} from 'react';
import {api} from './api';
import {repositoryScope} from './providerPullRequests';
import type {ProviderPullFile} from './providerActions';
import {CommitFileList} from './CommitFileList';
import {DiffView} from './DiffView';
/** Reuse the existing changed-files and diff UI. Provider paths never become local paths. */
export function ProviderPullFiles({repository,number,head,base,disabled,onBusy,onStale,active=false,statistics}:{statistics?:ChangeStatistics;repository:string;number:number;head:string;base:string;disabled:boolean;onBusy:(busy:boolean)=>void;onStale:()=>void;active?:boolean}){
 const [files,setFiles]=useState<ProviderPullFile[]>([]),[selected,setSelected]=useState<string|null>(null);
 const [viewed,setViewed]=useState<Set<string>>(()=>new Set()),[onlyUnviewed,setOnlyUnviewed]=useState(false);
 const [loaded,setLoaded]=useState(false),[next,setNext]=useState<number|null>(null),[limited,setLimited]=useState(false);
 const [busy,setBusy]=useState(false),[error,setError]=useState(''),[message,setMessage]=useState('');
 const pending=useRef(false);
 async function load(page:number){
  if(disabled||pending.current)return;
  pending.current=true;setBusy(true);onBusy(true);setError('');setMessage('');
  try{
   const result=await api.githubPullFiles(...repositoryScope(repository),number,page,head,base);
   if(!result){setMessage('File loading cancelled.');return;}
   const previous=page===1?[]:files;
   if(result.files.some(file=>previous.some(old=>old.path===file.path))){setFiles([]);setSelected(null);onStale();return;}
   setFiles([...previous,...result.files]);setNext(result.nextPage);setLimited(result.limitReached);setLoaded(true);
   if(page===1){setSelected(result.files.some(file=>file.path===selected)?selected:result.files[0]?.path??null);setViewed(previous=>new Set(result.files.filter(file=>previous.has(file.path)&&files.some(old=>old.path===file.path&&old.patch===file.patch&&old.status===file.status&&old.additions===file.additions&&old.deletions===file.deletions)).map(file=>file.path)));}
  }catch(e){
   const message=String(e);
   if(message.includes('PROVIDER_SNAPSHOT_CHANGED')){setFiles([]);setSelected(null);onStale();}
   else setError(message);
  }finally{pending.current=false;setBusy(false);onBusy(false);}
 }
 useInitialVisibleRead(active,disabled||loaded,()=>load(1));
 const visibleFiles=onlyUnviewed?files.filter(file=>!viewed.has(file.path)):files;
 const toggleViewed=(path:string)=>setViewed(previous=>{const next=new Set(previous);if(next.has(path))next.delete(path);else next.add(path);return next;});
 const file=files.find(file=>file.path===selected);
 return <section aria-label="Pull request changed files">
  <h3>Files changed</h3>
  {statistics&&<ChangeStats value={statistics}/>}
  <button disabled={disabled||busy} onClick={()=>void load(1)}>{loaded?'Reload changed files':'Load changed files'}</button>
  {busy&&<p role="status">Loading changed files…</p>}{message&&<p role="status">{message}</p>}{error&&<p role="alert">{error}</p>}
  {loaded&&<><div className="setting-row"><p>{files.length} files loaded · {viewed.size} viewed{next!==null?' · more available':''}.</p><label><input type="checkbox" checked={onlyUnviewed} onChange={event=>setOnlyUnviewed(event.target.checked)}/>Show unviewed files only</label><button disabled={disabled||busy||!viewed.size} onClick={()=>setViewed(new Set())}>Reset viewed files</button></div><p className="history-base">Viewed marks apply to this loaded review only. They are not submitted to GitHub.</p>
   {limited&&<p role="status">GitHub exposes at most 3,000 changed files. This list may be incomplete.</p>}
   <div className="compare-content"><nav aria-label="Pull request files"><CommitFileList files={visibleFiles.map(file=>({id:file.path,path:file.path,status:file.status,staged:false,unstaged:false}))} busy={disabled||busy} selected={selected??undefined} onSelect={file=>setSelected(file.id)} emptyMessage={onlyUnviewed?'All loaded files have been viewed.':'No changed files returned.'}/></nav>
    <section aria-label="Pull request file diff"><DiffFileNavigator files={visibleFiles.map(file=>({key:file.path,path:file.path,label:file.path}))} selected={selected??''} busy={disabled||busy} scope={onlyUnviewed?'Loaded unviewed files':'All loaded files'} onSelect={setSelected}/>{file&&<><div className="provider-file-heading"><h4>{file.previousPath?`${file.previousPath} → ${file.path}`:file.path}</h4><CopyTextButton text={file.path} label="Copy file path"/><span>+{file.additions} −{file.deletions}</span><label><input type="checkbox" disabled={disabled||busy} checked={viewed.has(file.path)} onChange={()=>toggleViewed(file.path)}/>Mark as viewed</label></div>{onlyUnviewed&&viewed.has(file.path)&&<p className="history-base">This file is now viewed and hidden from the unviewed list. Select another file or turn off the filter.</p>}
     {file.patch===null?<p>GitHub did not provide a text patch for this file. Binary files and omitted patches cannot be displayed here.</p>:<><p className="history-base">GitHub patch preview; large patches may be incomplete. This preview does not modify local files.</p><DiffView key={file.path} diff={{text:file.patch,truncated:true}} truncationNotice="Provider patch completeness is not guaranteed. Search covers the displayed preview only." busy={disabled||busy}/></>}
    </>}</section>
   </div>
   {next!==null&&<button disabled={disabled||busy} onClick={()=>void load(next)}>Load more changed files</button>}
  </>}
 </section>;
}
