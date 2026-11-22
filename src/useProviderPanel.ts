import {useProviderNavigation} from './ProviderNavigation';
import {useCallback,useEffect,useRef,useState} from 'react';
export type ProviderNavigationGuard=(proceed:(refresh:boolean)=>void)=>void;
export type RegisterProviderNavigationGuard=(guard:ProviderNavigationGuard|null)=>void;
export function useProviderPanel<T extends {repository:string;number?:number}>(onRefresh:(panel:T)=>void){
 const [panel,setPanel]=useState<T|null>(null);
 const navigation=useProviderNavigation();
 const identity=useRef(Symbol('provider panel'));
 const guard=useRef<ProviderNavigationGuard|null>(null);
 const registerNavigationGuard=useCallback<RegisterProviderNavigationGuard>(value=>{guard.current=value;},[]);
 useEffect(()=>{
  if(!navigation||!panel)return;
  return navigation.register(identity.current,proceed=>{
   const leave=(refresh:boolean)=>{if(refresh)onRefresh(panel);setPanel(null);proceed();};
   if(guard.current)guard.current(leave);else leave(false);
  });
 });
 const requestPanel=(next:T)=>{
  if(panel&&JSON.stringify(panel)===JSON.stringify(next))return;
  const proceed=(refresh:boolean)=>{if(refresh&&panel)onRefresh(panel);setPanel(next);};
  if(panel&&guard.current)guard.current(proceed);else proceed(false);
 };
 return {panel,setPanel,requestPanel,registerNavigationGuard};
}
