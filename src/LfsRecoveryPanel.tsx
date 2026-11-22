import {useEffect,useRef,useState} from 'react';
import {collectLfsIndex,type LfsIndexCatalog,type LfsScanProgress} from './lfsIndexCatalog';
import {LfsCheckoutPreview} from './LfsCheckoutPreview';
import {LfsObjectPaths} from './LfsObjectPaths';
import {LfsBatchTransferPanel} from './LfsBatchTransferPanel';
import {api} from './api';
import {X} from './OriginalIcons';
import {LfsUploadRecordView} from './LfsUploadRecordView';
import {LfsTransferForm} from './LfsTransferForm';
import {LfsPointerNotice} from './LfsPointerNotice';
import {inspectLfsPointer,type LfsPointer} from './lfsPointer';
import {useInitialVisibleRead} from './useInitialVisibleRead';
import type {Snapshot} from './types';
import type {LfsIndexPage,LfsUploadRecord} from './lfsTransfer';
export function LfsRecoveryPanel({snapshot,busy,onRun,onClose,initialTransfer}:{initialTransfer?:import("./lfsTransfer").LfsTransferMode;snapshot:Snapshot;busy:boolean;onRun:(task:()=>Promise<void>)=>Promise<void>;onClose:()=>void}){
 const [record,setRecord]=useState<LfsUploadRecord|null>(null),[loaded,setLoaded]=useState(false),[recordError,setRecordError]=useState('');
 const [catalog,setCatalog]=useState<LfsIndexCatalog|null>(null),[scanProgress,setScanProgress]=useState<LfsScanProgress|null>(null),[scanning,setScanning]=useState(false);
 const scanCancelled=useRef(false);
 const [objectOffset,setObjectOffset]=useState(0);
 const [page,setPage]=useState<LfsIndexPage|null>(null),[query,setQuery]=useState('');
 const [selected,setSelected]=useState<{path:string;pointer:LfsPointer}|null>(null),[object,setObject]=useState<{state:string;message:string}>();
 const [error,setError]=useState(''),[message,setMessage]=useState(''),[pending,setPending]=useState(false);
 const active=useRef(false),mounted=useRef(true);
 useEffect(()=>{mounted.current=true;return()=>{mounted.current=false;scanCancelled.current=true;};},[]);
 const readRecord=async()=>{
  try{const result=await api.lfsUploadRecord(snapshot);if(mounted.current){setRecord(result);setLoaded(true);setRecordError('');}}
  catch(reason){if(mounted.current){setRecordError(String(reason));setLoaded(true);}}
 };
 const readPage=async(offset:number)=>{
  const result=await api.lfsIndex(snapshot,offset);
  if(mounted.current){setPage(result);setCatalog(null);setScanProgress(null);setSelected(null);setObject(undefined);}
 };
 const perform=async<T,>(task:()=>Promise<T>):Promise<T>=>{
  if(busy||active.current)throw new Error('Another operation is running.');
  active.current=true;setPending(true);setError('');setMessage('');
  const result:{value?:T;completed:boolean}={completed:false};
  try{await onRun(async()=>{result.value=await task();result.completed=true;});if(!result.completed)throw new Error('Operation did not start. Reload when the current operation finishes.');return result.value as T;}
  catch(reason){if(mounted.current)setError(String(reason));throw reason;}
  finally{active.current=false;if(mounted.current)setPending(false);}
 };
 const execute=(task:()=>Promise<void>)=>perform(task).catch(()=>{});
 useInitialVisibleRead(true,busy,()=>initialTransfer?scan():execute(async()=>{await readRecord();await readPage(0);}),snapshot.repoId);
 const inspectRemote=(id:string)=>perform(async()=>{
  const result=await api.inspectLfsUploadRemote(snapshot,id);
  if(mounted.current)setMessage(result===null?'Server check cancelled.':result.present?'Server reported object present.':'Server requested upload; no bytes were sent.');
  if(result)await readRecord();return result;
 });
 const review=(id:string)=>perform(async()=>{
  const reviewed=await api.acknowledgeLfsUpload(snapshot,id);
  if(mounted.current)setMessage(reviewed?'Local outcome acknowledged. No remote object was changed.':'Review cancelled.');
  if(reviewed)await readRecord();return reviewed;
 });
 const inspectSelected=()=>perform(async()=>{
  if(!selected)throw new Error('No object selected.');
  const result=await api.inspectLfsCache(snapshot,selected.pointer.oid,selected.pointer.size);
  if(mounted.current)setObject(result);await readRecord();
 });
 const choose=(entry:{path:string;text:string})=>execute(async()=>{
  const pointer=inspectLfsPointer(entry.text);if(!pointer)throw new Error('Invalid LFS pointer. Reload the index.');
  const result=await api.inspectLfsCache(snapshot,pointer.oid,pointer.size);
  if(mounted.current){setSelected({path:entry.path,pointer});setObject(result);}
 });
 const scan=()=>execute(async()=>{
  scanCancelled.current=false;setScanning(true);setCatalog(null);setScanProgress(null);setSelected(null);setObject(undefined);
  try{await readRecord();const result=await collectLfsIndex(offset=>api.lfsIndex(snapshot,offset),()=>scanCancelled.current||!mounted.current,progress=>{if(mounted.current)setScanProgress(progress);});if(mounted.current){setCatalog(result);setObjectOffset(0);setMessage('Complete current-index scan. Objects are grouped by SHA-256; no network transfer was performed.');}}
  catch(reason){if(String(reason).includes('LFS_INDEX_SCAN_CANCELLED')){if(mounted.current)setMessage('Scan cancelled; no complete inventory was produced. Reload objects to return to the paged index view.');}else throw reason;}
  finally{if(mounted.current)setScanning(false);}
 });
 const blocked=busy||pending;
 const matchingObjects=catalog?.objects.filter(object=>object.paths.some(path=>path.toLocaleLowerCase().includes(query.toLocaleLowerCase())))??[];
 const entries=page?.entries.filter(entry=>entry.path.toLocaleLowerCase().includes(query.toLocaleLowerCase()))??[];
 return <div className="modal-backdrop"><section className="modal" role="dialog" aria-modal="true" aria-label="LFS objects and recovery"><header className="modal-header"><h2>LFS · {snapshot.name}</h2><button disabled={blocked} data-modal-close aria-label="Close LFS" onClick={onClose}><X size={18}/></button></header>
  <p>Objects below come from regular stage-zero files in the current index. Unstaged files, conflict stages and other revisions are not included.</p>
  <input className="wide-input" aria-label={catalog?"Filter all indexed LFS paths":"Filter LFS paths on this page"} placeholder={catalog?"Filter indexed LFS paths":"Filter paths on this page"} value={query} disabled={blocked} onChange={event=>{setQuery(event.target.value);setObjectOffset(0);}}/>
  <div className="setting-row"><button disabled={blocked} onClick={()=>void scan()}>Scan whole index</button>{scanning&&<button onClick={()=>{scanCancelled.current=true;setMessage('Stopping after the current index page…');}}>Stop scan</button>}{catalog&&<button disabled={blocked} onClick={()=>{setCatalog(null);setScanProgress(null);setSelected(null);setObject(undefined);}}>Return to pages</button>}</div>
  {scanProgress&&<p role="status">Scanned {scanProgress.scanned}/{scanProgress.total} index entries · {scanProgress.objects} unique objects{scanning?' · scanning…':catalog?' · complete':' · incomplete'}</p>}
  {catalog&&<section aria-label="Complete current-index LFS inventory"><p>{catalog.pointerPaths} pointer paths reference {catalog.objects.length} unique objects. Group details use the first pointer path.</p>{catalog.skippedNonUtf8>0&&<p>{catalog.skippedNonUtf8} pointer paths could not be displayed as UTF-8.</p>}<div className="selected-file-review">{matchingObjects.slice(objectOffset,objectOffset+200).map(object=><div key={object.oid}><button className="branch-item" disabled={blocked} aria-pressed={selected?.pointer.oid===object.oid} onClick={()=>void choose(object)}>{object.path} · {object.size} bytes{object.paths.length>1?` · ${object.paths.length} paths`:''}</button>{object.paths.length>1&&<LfsObjectPaths paths={object.paths} query={query}/>}</div>)}</div>{!matchingObjects.length&&<p>No matching indexed LFS objects.</p>}<div className="setting-row"><button disabled={blocked||objectOffset===0} onClick={()=>setObjectOffset(Math.max(0,objectOffset-200))}>Previous objects</button><span>{Math.min(objectOffset+200,matchingObjects.length)}/{matchingObjects.length} objects</span><button disabled={blocked||objectOffset+200>=matchingObjects.length} onClick={()=>setObjectOffset(objectOffset+200)}>Next objects</button></div></section>}
  {!catalog&&page&&<><p>Index entries {page.totalIndexEntries? page.offset+1:0}–{page.nextOffset??page.totalIndexEntries} of {page.totalIndexEntries}; {page.entries.length} LFS pointers on this page.</p><div className="selected-file-review">{entries.map(entry=><button key={entry.path} className="branch-item" disabled={blocked} aria-pressed={selected?.path===entry.path} onClick={()=>void choose(entry)}>{entry.path}</button>)}</div>{!entries.length&&<p>No matching LFS pointers on this page.</p>}{page.skippedNonUtf8>0&&<p>{page.skippedNonUtf8} pointer paths could not be displayed as UTF-8.</p>}<div className="setting-row"><button disabled={blocked||page.offset===0} onClick={()=>void execute(()=>readPage(Math.max(0,page.offset-1000)))}>Previous page</button><button disabled={blocked||page.nextOffset===null} onClick={()=>{if(page.nextOffset!==null)void execute(()=>readPage(page.nextOffset!));}}>Next page</button></div></>}
  {catalog&&<LfsBatchTransferPanel objects={catalog.objects} initialMode={initialTransfer} disabled={blocked} uploadBlocked={!!recordError||record?.status==='pending'||record?.status==='accepted'&&record.requiresVerify} onRun={task=>perform(async()=>{try{await task();}finally{await readRecord();}})} onTransfer={(object,endpoint,id,mode)=>(mode==='upload'?api.uploadLfsObject:api.downloadLfsObject)(snapshot,object.oid,object.size,endpoint,id)} onCancel={api.cancelLfsDownload}/>}
  {selected&&<section aria-label="Selected LFS object"><h3>{selected.path}</h3><LfsPointerNotice pointer={selected.pointer} object={object}/><LfsCheckoutPreview key={selected.path+':'+selected.pointer.oid} disabled={blocked} onPreview={()=>perform(()=>api.previewLfsCheckout(snapshot,selected.path,selected.pointer.oid,selected.pointer.size))}/><LfsTransferForm key={selected.path+':'+selected.pointer.oid+':'+selected.pointer.size} pointer={selected.pointer} disabled={blocked} uploadRecord={record??undefined} uploadError={recordError||undefined} onCancel={api.cancelLfsDownload} onInspect={inspectSelected} onInspectRemote={inspectRemote} onReview={review} onTransfer={(pointer,endpoint,requestId,mode)=>perform(()=>(mode==='upload'?api.uploadLfsObject:api.downloadLfsObject)(snapshot,pointer.oid,pointer.size,endpoint,requestId))}/></section>}
  {!selected&&record&&<LfsUploadRecordView record={record} busy={blocked||!!error||!!recordError} onInspect={()=>void inspectRemote(record.id).catch(()=>{})} onReview={()=>void review(record.id).catch(()=>{})}/>}
  {recordError&&<p role="alert">Upload record could not be read: {recordError}. Uploads remain blocked until the record is repaired.</p>}
  {loaded&&!record&&!recordError&&<p>No saved LFS upload record.</p>}
  {!loaded&&!error&&<p role="status">Loading upload record…</p>}
  {error&&<p role="alert">{error} Displayed data may be stale. For a stale repository revision, close this window and refresh the repository.</p>}{message&&<p role="status">{message}</p>}
  <div className="setting-row"><button disabled={blocked} onClick={()=>void execute(async()=>{await readRecord();await readPage(0);})}>Reload objects and record</button><button disabled={blocked} data-modal-close onClick={onClose}>Close</button></div>
 </section></div>;
}
