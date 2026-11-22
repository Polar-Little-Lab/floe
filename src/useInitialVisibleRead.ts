import {useEffect,useRef} from 'react';
/** One automatic read per mounted scope; cancellation/failure requires explicit retry. */
export function useInitialVisibleRead(active:boolean,disabled:boolean,read:()=>Promise<void>,scope='mounted'){
 const attempted=useRef<string|null>(null),latest=useRef(read);latest.current=read;
 useEffect(()=>{
  if(!active||disabled||attempted.current===scope)return;
  let current=true;
  queueMicrotask(()=>{if(!current||attempted.current===scope)return;attempted.current=scope;void latest.current();});
  return()=>{current=false;};
 },[active,disabled,scope]);
}
