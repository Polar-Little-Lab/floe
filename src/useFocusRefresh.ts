import {useEffect,useRef} from 'react';
/** Coalesce focus signals, keeping one pending refresh until foreground work is idle. */
export function useFocusRefresh(repositoryId:string,enabled:boolean,refresh:()=>Promise<void>,blocked:()=>boolean){
 const latest=useRef({refresh,blocked});latest.current={refresh,blocked};
 useEffect(()=>{
  if(!enabled)return;
  let stopped=false,pending=false,running=false;let timer:ReturnType<typeof setTimeout>|undefined;
  const schedule=()=>{if(!stopped&&!timer&&pending&&!running&&!document.hidden)timer=setTimeout(flush,300);};
  const flush=()=>{
   timer=undefined;if(stopped||!pending||document.hidden)return;
   if(!document.hasFocus())return;
   if(latest.current.blocked()||document.querySelector('[aria-modal="true"]')){schedule();return;}
   pending=false;running=true;
   // The caller owns error presentation. Never retry a failed refresh automatically.
   void latest.current.refresh().catch(()=>{}).finally(()=>{running=false;schedule();});
  };
  const request=()=>{pending=true;schedule();};
  const visibility=()=>{if(!document.hidden)request();else if(timer){clearTimeout(timer);timer=undefined;}};
  window.addEventListener('focus',request);document.addEventListener('visibilitychange',visibility);
  return()=>{stopped=true;if(timer)clearTimeout(timer);window.removeEventListener('focus',request);document.removeEventListener('visibilitychange',visibility);};
 },[repositoryId,enabled]);
}
