import {createContext,useCallback,useContext,useMemo,useRef,type ReactNode} from 'react';
type Guard=(proceed:()=>void)=>void;
type Navigation={register:(id:symbol,guard:Guard)=>()=>void;request:(proceed:()=>void)=>void};
const Context=createContext<Navigation|null>(null);
export function ProviderNavigation({children}:{children:ReactNode}){
 const guards=useRef(new Map<symbol,Guard>());
 const register=useCallback((id:symbol,guard:Guard)=>{guards.current.set(id,guard);return()=>{if(guards.current.get(id)===guard)guards.current.delete(id);};},[]);
 const request=useCallback((proceed:()=>void)=>{
  const entries=[...guards.current.keys()];
  const next=(index:number)=>{if(index===entries.length){proceed();return;}const guard=guards.current.get(entries[index]);if(!guard){next(index+1);return;}guard(()=>next(index+1));};
  next(0);
 },[]);
 const value=useMemo(()=>({register,request}),[register,request]);
 return <Context.Provider value={value}>{children}</Context.Provider>;
}
export function useProviderNavigation(){return useContext(Context);}
