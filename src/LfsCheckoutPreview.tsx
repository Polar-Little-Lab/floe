import {useEffect,useRef,useState} from 'react';
import type {LfsCheckoutPreview as Preview} from './lfsTransfer';
export function LfsCheckoutPreview({disabled,onPreview}:{disabled:boolean;onPreview:()=>Promise<Preview>}){
 const [preview,setPreview]=useState<Preview|null>(null),[error,setError]=useState(''),[pending,setPending]=useState(false);
 const active=useRef(false),mounted=useRef(true);
 useEffect(()=>{mounted.current=true;return()=>{mounted.current=false;};},[]);
 const inspect=async()=>{
  if(disabled||active.current)return;
  active.current=true;setPending(true);setError('');setPreview(null);
  try{const result=await onPreview();if(mounted.current)setPreview(result);}catch(reason){if(mounted.current)setError(String(reason));}
  finally{active.current=false;if(mounted.current)setPending(false);}
 };
 return <section className="lfs-pointer-notice" aria-label="LFS checkout preview"><button disabled={disabled||pending} onClick={()=>void inspect()}>{pending?'Inspecting checkout…':'Preview checkout…'}</button>
  {preview&&<><p>{preview.path} · {preview.size} bytes · {preview.executable?'executable':'regular file'}</p><p>Index pointer blob: <code>{preview.indexOid}</code></p><p role="status">{preview.worktreeState==='pointer'?'Working file exactly matches the indexed pointer.':preview.worktreeState==='missing'?'Working file is missing.':preview.worktreeState==='different'?'Working file differs from the indexed pointer. It may contain hydrated data or local edits; it must not be overwritten.':'Working path is not a supported regular file.'}</p><p>{preview.cache.message}</p></>}
  {error&&<p role="alert">{error}</p>}
  <p>This preview does not replace files. Transactional checkout and recovery are still being implemented; no checkout action is enabled yet.</p>
 </section>;
}
