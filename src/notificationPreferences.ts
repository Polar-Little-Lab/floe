import {useEffect,useState} from 'react';
export type NotificationPreferences={showBadge:boolean;showMessages:boolean};
const key='branchdesk.notifications.v1';
const defaults:NotificationPreferences={showBadge:true,showMessages:true};
function read():NotificationPreferences{try{const value=JSON.parse(localStorage.getItem(key)??'null');return {showBadge:typeof value?.showBadge==='boolean'?value.showBadge:true,showMessages:typeof value?.showMessages==='boolean'?value.showMessages:true};}catch{return {...defaults};}}
export function useNotificationPreferences(){
 const [preferences,setPreferences]=useState(read),[warning,setWarning]=useState('');
 useEffect(()=>{const sync=(event:StorageEvent)=>{if(event.storageArea===localStorage&&(event.key===key||event.key===null)){setPreferences(read());setWarning('');}};window.addEventListener('storage',sync);return()=>window.removeEventListener('storage',sync);},[]);
 const change=(value:NotificationPreferences)=>{try{localStorage.setItem(key,JSON.stringify(value));setPreferences(value);setWarning('');}catch{setWarning('Notification preferences could not be saved. Your previous settings are still active.');}};
 return {preferences,change,warning};
}
