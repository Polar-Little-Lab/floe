import {useEffect,useRef,useState} from 'react';
import {api,native} from './api';
import type {DiffResult} from './types';
export function SavePatchButton({diff,busy}:{diff:DiffResult|null;busy:boolean}){
 const [pending,setPending]=useState(false);const [status,setStatus]=useState('');const request=useRef(0);
 useEffect(()=>{request.current++;setPending(false);setStatus('');return()=>{request.current++;};},[diff]);
 const enabled=native&&!busy&&!pending&&!!diff&&!diff.truncated&&diff.text.startsWith('diff --git ');
 const save=async()=>{if(!enabled||!diff)return;const token=++request.current;setPending(true);setStatus('');try{const path=await api.exportPatch(diff.text,diff.truncated);if(token===request.current)setStatus(path?'Patch saved':'Save cancelled');}catch(error){if(token===request.current)setStatus(String(error));}finally{if(token===request.current)setPending(false);}};
 return <span className="copy-text-control"><button disabled={!enabled} title="Save displayed Git diff to a new .patch file. Binary payloads are not included; existing files are not replaced." onClick={()=>void save()}>{pending?'Saving…':'Save new patch…'}</button>{status&&<small role="status">{status}</small>}</span>;
}
