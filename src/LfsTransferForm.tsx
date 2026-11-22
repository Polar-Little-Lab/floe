import {useEffect,useRef,useState} from 'react';
import type {LfsPointer} from './lfsPointer';
import type {LfsTransferResult,LfsUploadRecord,LfsUploadObservation} from './lfsTransfer';
import {LfsUploadRecordView} from './LfsUploadRecordView';
import {FilterSelect} from './ui/FilterSelect';

export function LfsTransferForm({pointer,disabled,onTransfer,onCancel,onInspect,uploadRecord,uploadError,onReview,onInspectRemote}:{pointer:LfsPointer;disabled:boolean;uploadRecord?:LfsUploadRecord;uploadError?:string;onInspectRemote?:(id:string)=>Promise<LfsUploadObservation|null>;onReview?:(id:string)=>Promise<boolean>;onInspect?:()=>Promise<void>;onTransfer:(pointer:LfsPointer,endpoint:string,requestId:string,mode:'download'|'upload')=>Promise<LfsTransferResult>;onCancel?:(requestId:string)=>Promise<boolean>}) {
 const [expanded,setExpanded]=useState(false);
 const [endpoint,setEndpoint]=useState('');
 const [mode,setMode]=useState<'download'|'upload'>('download');
 const [uncertain,setUncertain]=useState(false);
 const [remoteReviewed,setRemoteReviewed]=useState(false);
 const [pending,setPending]=useState(false);
 const [reviewing,setReviewing]=useState(false);
 const unresolved=uploadRecord?.status==='pending'||(uploadRecord?.status==='accepted'&&uploadRecord.requiresVerify);
 const uploadBlocked=!!uploadError||unresolved;
 const [inspecting,setInspecting]=useState(false);
 const [attempted,setAttempted]=useState(false);
 const [message,setMessage]=useState('');
 const [error,setError]=useState('');
 const [cancelRequested,setCancelRequested]=useState(false);
 const [cancelling,setCancelling]=useState(false);
 const request=useRef<string|null>(null);
 const cancellingRef=useRef(false);
 const active=useRef(false),mounted=useRef(true);
 useEffect(()=>{mounted.current=true;return()=>{mounted.current=false;};},[]);
 const submitTransfer=async()=>{
  if(disabled||active.current||attempted||(mode==='upload'&&uploadBlocked))return;
  const address=endpoint.trim();
  try{const url=new URL(address);if(url.protocol!=='https:'||url.username||url.password||url.search||url.hash)throw new Error();}
  catch{setError('Enter an HTTPS LFS server URL without credentials, query or fragment.');return;}
  const requestId=crypto.randomUUID();request.current=requestId;
  active.current=true;setPending(true);setAttempted(true);setError('');setMessage('');setCancelRequested(false);
  try{
   const result=await onTransfer(pointer,address,requestId,mode);
   if(mounted.current){
    if(result===null||result==='cancelled'){setMessage(mode==='upload'?'Upload cancelled before sending object bytes.':result===null?'Download cancelled before transfer.':'Download cancelled. No new object was published by this request.');setAttempted(false);}
    else if(result==='created'||result==='alreadyPresent')setMessage(result==='created'?'Object downloaded; size and SHA-256 verified. Working files are unchanged.':'Existing cached object verified; working files are unchanged.');
    else if(result==='remotePresent')setMessage('The server reports that this object is already present. No object bytes were uploaded.');
    else if(result==='uploadedVerified')setMessage('Upload accepted and server verification completed. Git refs and working files are unchanged.');
    else if(result==='uploaded')setMessage('Upload accepted. The server did not request a separate verification step. Git refs and working files are unchanged.');
    else {setError('Unexpected transfer result. Inspect local and remote state before retrying.');setUncertain(mode==='upload');}
   }
  }catch(reason){if(mounted.current){setUncertain(mode==='upload');setRemoteReviewed(false);setError(String(reason)+(mode==='upload'?' Remote state must be checked before retrying.':' Refresh cache status before retrying.'));}}
  finally{
   request.current=null;
   if(mode==='upload'&&onInspect&&mounted.current){try{await onInspect();}catch{if(mounted.current)setMessage('Refresh cache status to inspect the durable upload record.');}}
   active.current=false;if(mounted.current)setPending(false);
  }
 };
 const inspect=async()=>{
  if(disabled||active.current||!onInspect)return;
  active.current=true;setInspecting(true);
  try{await onInspect();if(mounted.current){if(!uncertain){setAttempted(false);setError('');}setMessage(uncertain?'Local cache refreshed. This does not determine the remote upload outcome.':'Cache inspection refreshed. Review its status above before transferring again.');}}
  catch(reason){if(mounted.current)setError('Cache inspection failed: '+String(reason));}
  finally{active.current=false;if(mounted.current)setInspecting(false);}
 };
 const inspectRemote=async()=>{
  if(disabled||active.current||!uploadRecord||!onInspectRemote)return;
  active.current=true;setReviewing(true);setError('');setMessage('');
  try{const observation=await onInspectRemote(uploadRecord.id);if(mounted.current){setMessage(observation===null?'Server check cancelled.':observation.present?'Server reports the object already exists. This is an availability observation, not an independent hash verification.':'Server requests an upload for this object. No object bytes were sent.');}}
  catch(reason){if(mounted.current)setError('Server check failed: '+String(reason));}
  finally{active.current=false;if(mounted.current)setReviewing(false);}
 };
 const review=async()=>{
  if(disabled||active.current||!uploadRecord||!onReview)return;
  active.current=true;setReviewing(true);
  try{if(await onReview(uploadRecord.id)){if(mounted.current){setUncertain(false);setRemoteReviewed(false);setAttempted(false);setError('');setMessage('Previous outcome acknowledged. No remote object was changed.');}}}
  catch(reason){if(mounted.current)setError(String(reason));}
  finally{active.current=false;if(mounted.current)setReviewing(false);}
 };
 const cancel=async()=>{
  const requestId=request.current;
  if(!requestId||!onCancel||cancellingRef.current||cancelRequested)return;
  cancellingRef.current=true;setCancelling(true);
  try{const accepted=await onCancel(requestId);if(mounted.current&&request.current===requestId){setCancelRequested(accepted);setMessage(accepted?'Cancellation requested. Waiting for the current prompt or network read to finish.':'Transfer has not started or has already finished.');}}
  catch(reason){if(mounted.current&&request.current===requestId)setMessage('Cancellation request failed: '+String(reason));}
  finally{cancellingRef.current=false;if(mounted.current)setCancelling(false);}
 };
 return <section className="lfs-pointer-notice" aria-label="Transfer LFS object">
  <button disabled={disabled||pending||inspecting||reviewing} aria-expanded={expanded} onClick={()=>setExpanded(!expanded)}>Transfer LFS object…</button>
  {onInspect&&<button disabled={disabled||pending||inspecting||reviewing} onClick={()=>void inspect()}>{inspecting?'Inspecting cache…':'Refresh cache status'}</button>}
  {uploadError&&<p role="alert">{uploadError}</p>}
  {uploadRecord&&<LfsUploadRecordView record={uploadRecord} busy={disabled||pending||inspecting||reviewing} onInspect={onInspectRemote?()=>void inspectRemote():undefined} onReview={onReview?()=>void review():undefined}/>}
  {expanded&&<form onSubmit={event=>{event.preventDefault();void submitTransfer();}}>
   <FilterSelect label="LFS operation" placeholder="Choose an operation" value={mode} options={[{value:'download',label:'Download to cache'},{value:'upload',label:'Upload cached object'}]} disabled={disabled||pending||inspecting||reviewing||attempted} onChange={value=>{if(value==='download'||value==='upload')setMode(value);}}/>
   <p>{mode==='upload'?'Upload this object from the standard repository cache. Its size and hash are checked in a temporary snapshot before transfer; additional temporary disk space equal to the object size is required.':'Download this object to the standard repository cache.'} Enter your LFS server address, typically ending in /info/lfs. Custom storage settings are not used.</p>
   <label>LFS server URL <input type="url" required placeholder="https://host/owner/repository.git/info/lfs" value={endpoint} disabled={disabled||pending||inspecting||reviewing||attempted} onChange={event=>{setEndpoint(event.target.value);setError('');}}/></label>
   <p>The app will confirm the server and object destination in native dialogs. Enter credentials only in those dialogs.</p>
   <button type="submit" disabled={disabled||pending||inspecting||reviewing||attempted||!endpoint.trim()||(mode==='upload'&&uploadBlocked)}>{pending?(mode==='upload'?'Uploading…':'Downloading…'):(mode==='upload'?'Upload cached object':'Download to cache')}</button>
   {pending&&<><p role="status">Waiting for confirmation or transfer. Cancellation takes effect between reads; an open native dialog must be dismissed and a blocked network read may take until its timeout.</p>{onCancel&&<button type="button" disabled={cancelling||cancelRequested} onClick={()=>void cancel()}>{cancelRequested?'Cancellation requested':cancelling?'Requesting cancellation…':'Cancel transfer'}</button>}</>}
  </form>}
  {uncertain&&!pending&&!uploadBlocked&&<div><label><input type="checkbox" checked={remoteReviewed} disabled={disabled||inspecting} onChange={event=>setRemoteReviewed(event.target.checked)}/>I checked the remote object state and want to allow a new upload request.</label><button disabled={disabled||inspecting||!remoteReviewed} onClick={()=>{setUncertain(false);setRemoteReviewed(false);setAttempted(false);setError('');setMessage('Retry enabled. The next request will check the server again before uploading.');}}>Enable upload retry</button></div>}
  {message&&<p role="status">{message}</p>}{error&&<p role="alert">{error}</p>}
 </section>;
}
