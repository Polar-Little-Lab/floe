import {useEffect,useMemo,useRef,useState} from 'react';
import type {LfsIndexObject} from './lfsIndexCatalog';
import type {LfsTransferResult,LfsTransferMode} from './lfsTransfer';
import {FilterSelect} from './ui/FilterSelect';
type Progress={state:'running'|'complete'|'stopped'|'failed';completed:number;created:number;present:number;uploaded:number;verified:number;path:string;detail:string};
export function LfsBatchTransferPanel({objects,disabled,onRun,onTransfer,onCancel,initialMode='download',uploadBlocked=false}:{objects:LfsIndexObject[];disabled:boolean;initialMode?:LfsTransferMode;uploadBlocked?:boolean;onRun:(task:()=>Promise<void>)=>Promise<void>;onTransfer:(object:LfsIndexObject,endpoint:string,id:string,mode:LfsTransferMode)=>Promise<LfsTransferResult>;onCancel:(id:string)=>Promise<boolean>}){
 const [endpoint,setEndpoint]=useState(''),[progress,setProgress]=useState<Progress|null>(null),[error,setError]=useState('');
 const [mode,setMode]=useState<LfsTransferMode>(initialMode);
 const [running,setRunning]=useState(false),[stopping,setStopping]=useState(false);
 const active=useRef(false),stopRequested=useRef(false),request=useRef<string|null>(null),mounted=useRef(true);
 const totalBytes=useMemo(()=>objects.reduce((sum,object)=>sum+BigInt(object.size),0n).toString(),[objects]);
 useEffect(()=>{mounted.current=true;return()=>{mounted.current=false;stopRequested.current=true;const id=request.current;if(id)void onCancel(id).catch(()=>{});};},[onCancel]);
 const start=async()=>{
  if(disabled||active.current||!objects.length||(mode==='upload'&&uploadBlocked))return;
  const address=endpoint.trim();
  try{const url=new URL(address);if(url.protocol!=='https:'||url.username||url.password||url.search||url.hash)throw new Error();}catch{setError('Enter an HTTPS LFS base URL without credentials, query or fragment.');return;}
  active.current=true;stopRequested.current=false;setRunning(true);setStopping(false);setError('');
  const next:Progress={state:'running',completed:0,created:0,present:0,uploaded:0,verified:0,path:'',detail:''};
  const report=()=>{if(mounted.current)setProgress({...next});};report();
  let started=false;
  try{await onRun(async()=>{
   started=true;
   for(const object of objects){
    if(stopRequested.current||!mounted.current){next.state='stopped';next.detail='Stopped before the next object.';report();return;}
    next.path=object.path;report();const id=crypto.randomUUID();request.current=id;
    try{
     const result=await onTransfer(object,address,id,mode);
     if(result===null||result==='cancelled'){next.state='stopped';next.detail='Current object cancelled. Remaining objects were not requested.';report();return;}
     if(mode==='download'){
      if(result!=='created'&&result!=='alreadyPresent')throw new Error('Unexpected LFS download result.');
      if(result==='created')next.created++;else next.present++;
     }else{
      if(result!=='uploaded'&&result!=='uploadedVerified'&&result!=='remotePresent')throw new Error('Unexpected LFS upload result.');
      if(result==='remotePresent')next.present++;else{next.uploaded++;if(result==='uploadedVerified')next.verified++;}
     }
     next.completed++;
     report();
    }catch(reason){next.state='failed';next.detail=String(reason);report();return;}
    finally{request.current=null;}
   }
   next.state='complete';next.path='';next.detail=mode==='download'?'All collected objects were verified in the local cache.':'All collected objects were accepted by the server or reported already present. Required verification callbacks completed.';report();
  });if(!started)throw new Error('The batch did not start. Another operation may be running.');}
  catch(reason){next.state='failed';next.detail=String(reason);report();}
  finally{active.current=false;request.current=null;if(mounted.current){setRunning(false);setStopping(false);}}
 };
 const stop=async()=>{
  stopRequested.current=true;setStopping(true);const id=request.current;
  if(id){try{await onCancel(id);}catch(reason){if(mounted.current)setError('Cancellation could not reach the current request: '+String(reason)+'. The queue will still stop before its next object.');}}
 };
 return <section className="lfs-pointer-notice" aria-label="Transfer indexed LFS objects"><h3>{mode==='upload'?'Push':'Fetch'} all indexed LFS objects</h3>
  <FilterSelect label="Batch LFS operation" placeholder="Choose operation" value={mode} disabled={disabled||running} options={[{value:'download',label:'Fetch indexed objects'},{value:'upload',label:'Push indexed objects'}]} onChange={value=>{if(value==='download'||value==='upload'){setMode(value);setProgress(null);setError('');}}}/>
  <p>{objects.length} unique objects · {totalBytes} declared bytes. Uses the complete current-index scan, including paths hidden by the filter. Each server/object destination still requires native confirmation. Working files and Git refs are unchanged.</p>
  {mode==='upload'&&<p>Each object must already exist in the local cache. A failed or uncertain upload stops the batch; review the durable upload record before retrying.</p>}
  {mode==='upload'&&uploadBlocked&&<p role="alert">The upload record requires review or repair before another upload can start.</p>}
  <label>LFS server URL <input type="url" placeholder="https://host/owner/repository.git/info/lfs" value={endpoint} disabled={disabled||running} onChange={event=>setEndpoint(event.target.value)}/></label>
  <div className="setting-row"><button disabled={disabled||running||!objects.length||!endpoint.trim()||(mode==='upload'&&uploadBlocked)} onClick={()=>void start()}>{progress?'Start new batch':mode==='upload'?'Push indexed objects':'Fetch indexed objects'}</button>{running&&<button disabled={stopping} onClick={()=>void stop()}>{stopping?'Stopping…':'Stop batch'}</button>}</div>
  {progress&&<div role="status"><p>{progress.state} · {progress.completed}/{objects.length} objects · {progress.created} downloaded · {progress.present} already present · {progress.uploaded} uploaded · {progress.verified} server-verified</p>{progress.path&&<p>{progress.path}</p>}<p>{progress.detail}</p>{progress.state!=='running'&&<p>Completed transfers are retained. A new batch starts from the current scan; it is never automatic and does not roll back earlier objects.</p>}</div>}
  {error&&<p role="alert">{error}</p>}
 </section>;
}
