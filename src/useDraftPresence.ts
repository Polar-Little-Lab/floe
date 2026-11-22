import {useEffect} from 'react';
export type DraftPresence=(id:string,dirty:boolean)=>void;
export function useDraftPresence(id:string,dirty:boolean,onChange?:DraftPresence){
 useEffect(()=>{onChange?.(id,dirty);return()=>onChange?.(id,false);},[id,dirty,onChange]);
}
