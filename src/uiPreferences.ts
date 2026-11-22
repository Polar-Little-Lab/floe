import {useEffect,useState} from 'react';
export const notificationLocations=['bottom-left','bottom-right','top-left','top-right'] as const;
export type NotificationLocation=typeof notificationLocations[number];
export type DateStyle='date'|'word'|'dateTime'|'verbose';
export type UIPreferences={locale:string;date:string;word:string;dateTime:string;verbose:string;toolbarLabels:boolean;notificationLocation:NotificationLocation};
export const defaultUIPreferences:UIPreferences={locale:'system',date:'MM/DD/YYYY',word:'MMMM D, YYYY',dateTime:'MM/DD/YYYY @ h:mm A',verbose:'MMMM D, YYYY h:mm A',toolbarLabels:true,notificationLocation:'bottom-left'};
export const dateLocales=['system','en-US','en-GB','ko-KR','ja-JP','de-DE','fr-FR'] as const;
const key='branchdesk.ui-preferences.v1',eventName='branchdesk-ui-preferences';
function read():UIPreferences{try{const value=JSON.parse(localStorage.getItem(key)??'null');const result={...defaultUIPreferences};if(!value)return result;for(const field of ['date','word','dateTime','verbose'] as const)if(typeof value[field]==='string'&&value[field].trim()&&value[field].length<=120)result[field]=value[field];if(dateLocales.includes(value.locale))result.locale=value.locale;if(typeof value.toolbarLabels==='boolean')result.toolbarLabels=value.toolbarLabels;if(notificationLocations.includes(value.notificationLocation))result.notificationLocation=value.notificationLocation;return result;}catch{return {...defaultUIPreferences};}}
export function useUIPreferences(){
 const [preferences,setPreferences]=useState(read),[warning,setWarning]=useState('');
 useEffect(()=>{const sync=()=>setPreferences(read());const storage=(event:StorageEvent)=>{if(event.storageArea===localStorage&&(event.key===key||event.key===null))sync();};window.addEventListener(eventName,sync);window.addEventListener('storage',storage);return()=>{window.removeEventListener(eventName,sync);window.removeEventListener('storage',storage);};},[]);
 const save=(value:UIPreferences)=>{try{localStorage.setItem(key,JSON.stringify(value));window.dispatchEvent(new Event(eventName));setWarning('');return true;}catch{setWarning('UI preferences could not be saved. Previous settings remain active.');return false;}};
 return {preferences,save,warning};
}
export function formatUIDate(value:number|string,preferences:UIPreferences,style:DateStyle='dateTime'){
 const date=new Date(value);if(!Number.isFinite(date.getTime()))return 'Unknown date';
 const locale=preferences.locale==='system'?undefined:preferences.locale;
 const pad=(n:number)=>String(n).padStart(2,'0');
 const month=(width:'long'|'short')=>new Intl.DateTimeFormat(locale,{month:width}).format(date);
 const parts:Record<string,string>={YYYY:String(date.getFullYear()),MMMM:month('long'),MMM:month('short'),MM:pad(date.getMonth()+1),M:String(date.getMonth()+1),DD:pad(date.getDate()),D:String(date.getDate()),HH:pad(date.getHours()),H:String(date.getHours()),hh:pad(date.getHours()%12||12),h:String(date.getHours()%12||12),mm:pad(date.getMinutes()),ss:pad(date.getSeconds()),A:date.getHours()<12?'AM':'PM',a:date.getHours()<12?'am':'pm'};
 return preferences[style].replace(/\[[^\]]*\]|YYYY|MMMM|MMM|MM|M|DD|D|HH|H|hh|h|mm|ss|A|a/g,token=>token.startsWith('[')?token.slice(1,-1):parts[token]);
}
