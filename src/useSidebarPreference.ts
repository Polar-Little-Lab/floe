import {useCallback,useEffect,useState} from 'react';
export const SIDEBAR_KEY='branchdesk-sidebar-collapsed';
function load(){try{const value=window.localStorage.getItem(SIDEBAR_KEY);return value===null||value==='false'?{collapsed:false,warning:''}:value==='true'?{collapsed:true,warning:''}:{collapsed:false,warning:'Saved sidebar layout was invalid. Toggle the sidebar to replace it.'};}catch{return {collapsed:false,warning:'Sidebar layout could not be read. Changes may last only for this session.'};}}
export function useSidebarPreference(beforeExternalCollapse:()=>void){
 const [state,setState]=useState(load);
 const choose=useCallback((collapsed:boolean)=>{let warning='';try{window.localStorage.setItem(SIDEBAR_KEY,String(collapsed));}catch{warning='Sidebar changed for this session, but could not be saved. Toggle it to retry.';}setState({collapsed,warning});},[]);
 useEffect(()=>{const changed=(event:StorageEvent)=>{if(event.key!==SIDEBAR_KEY&&event.key!==null)return;try{if(event.storageArea!==window.localStorage)return;}catch{return;}const next=load();if(next.collapsed)beforeExternalCollapse();setState(next);};window.addEventListener('storage',changed);return()=>window.removeEventListener('storage',changed);},[beforeExternalCollapse]);
 return {sidebarCollapsed:state.collapsed,sidebarWarning:state.warning,chooseSidebarCollapsed:choose};
}
