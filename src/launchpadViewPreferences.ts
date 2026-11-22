import {useEffect,useState} from 'react';
export type LaunchpadTablePreferences={age?:boolean;author:boolean;collaborators:boolean;repository:boolean;compact:boolean};
export const defaultLaunchpadTablePreferences:LaunchpadTablePreferences={age:true,author:true,collaborators:true,repository:true,compact:false};
const key='branchdesk.launchpad-table.v1';
function read():LaunchpadTablePreferences{try{const v=JSON.parse(localStorage.getItem(key)??'null');return Object.fromEntries(Object.entries(defaultLaunchpadTablePreferences).map(([name,fallback])=>[name,typeof v?.[name]==='boolean'?v[name]:fallback])) as LaunchpadTablePreferences;}catch{return {...defaultLaunchpadTablePreferences};}}
export function useLaunchpadViewPreferences(){
 const [value,setValue]=useState(read),[warning,setWarning]=useState('');
 useEffect(()=>{const sync=(event:StorageEvent)=>{if(event.storageArea===localStorage&&(event.key===key||event.key===null)){setValue(read());setWarning('');}};window.addEventListener('storage',sync);return()=>window.removeEventListener('storage',sync);},[]);
 const save=(next:LaunchpadTablePreferences)=>{try{localStorage.setItem(key,JSON.stringify(next));setValue(next);setWarning('');}catch{setWarning('View settings could not be saved. The previous layout is still active.');}};
 return {value,save,warning,reset:()=>save({...defaultLaunchpadTablePreferences})};
}
