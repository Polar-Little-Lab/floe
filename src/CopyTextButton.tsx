import {useEffect,useRef,useState} from 'react';
export function CopyTextButton({text,label,disabled=false,title}:{text:string;label:string;disabled?:boolean;title?:string}){
 const [pending,setPending]=useState(false);const [status,setStatus]=useState('');const request=useRef(0);
 useEffect(()=>{request.current++;setPending(false);setStatus('');return()=>{request.current++;};},[text]);
 const copy=async()=>{if(disabled||pending||!text)return;const token=++request.current;setPending(true);setStatus('');try{await navigator.clipboard.writeText(text);if(token===request.current)setStatus('Copied');}catch{if(token===request.current)setStatus('Clipboard unavailable');}finally{if(token===request.current)setPending(false);}};
 return <span className="copy-text-control"><button type="button" disabled={disabled||pending||!text} title={title} onClick={()=>void copy()}>{pending?'Copying...':label}</button>{status&&<small role="status">{status}</small>}</span>;
}
