import {useEffect,useRef,useState} from 'react';
import type {Snapshot} from './types';

/** Reads share App's operation gate with mutations; never retry a write here. */
export function useSidebarRead<T>(snapshot:Snapshot,enabled:boolean,busy:boolean,read:(snapshot:Snapshot)=>Promise<T>,run:(task:()=>Promise<void>)=>Promise<void>){
 const [retry,setRetry]=useState(0),[settled,setSettled]=useState(0);
 const key=`${snapshot.repoId}:${snapshot.revision}:${retry}`;
 const scope=useRef(key);scope.current=key;
 const mounted=useRef(true),pending=useRef(false);
 const latest=useRef({snapshot,read,run});latest.current={snapshot,read,run};
 const [result,setResult]=useState<{key:string;value:T|null;error:string}|null>(null);
 useEffect(()=>{mounted.current=true;return()=>{mounted.current=false;};},[]);
 useEffect(()=>{
  if(!enabled||busy||pending.current||result?.key===key)return;
  pending.current=true;
  const current=latest.current;
  void current.run(async()=>{
   if(!mounted.current||scope.current!==key)return;
   try{const value=await current.read(current.snapshot);if(mounted.current&&scope.current===key)setResult({key,value,error:''});}
   catch(reason){if(mounted.current&&scope.current===key)setResult({key,value:null,error:String(reason)});}
  }).finally(()=>{pending.current=false;if(mounted.current)setSettled(value=>value+1);});
 },[key,enabled,busy,result,settled]);
 return {value:result?.key===key?result.value:null,error:result?.key===key?result.error:'',reload:()=>setRetry(value=>value+1)};
}
