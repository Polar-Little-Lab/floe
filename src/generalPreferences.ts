import {useEffect,useState} from 'react';
export type LaunchpadView='pulls'|'issues'|'wips'|'snoozed'|'all'|'repositories';
export type GeneralPreferences={defaultBranch:string;launchpadView:LaunchpadView;rememberTabs:boolean;initialCommits?:number;lazyLoadCommits?:boolean;autoPrune?:boolean};
const key='branchdesk.general-preferences.v1';
export function readGeneralPreferences():GeneralPreferences {
 const defaults:GeneralPreferences={defaultBranch:'main',launchpadView:'pulls',rememberTabs:true,initialCommits:500,lazyLoadCommits:false,autoPrune:false};
 try {const v=JSON.parse(localStorage.getItem(key)??'null');if(!v)return defaults;return {autoPrune:typeof v.autoPrune==='boolean'?v.autoPrune:false,lazyLoadCommits:typeof v.lazyLoadCommits==='boolean'?v.lazyLoadCommits:false,initialCommits:validInitialCommits(v.initialCommits)?v.initialCommits:500,rememberTabs:typeof v.rememberTabs==='boolean'?v.rememberTabs:true,defaultBranch:typeof v.defaultBranch==='string'&&validDefaultBranch(v.defaultBranch)?v.defaultBranch:'main',launchpadView:['pulls','issues','wips','snoozed','all','repositories'].includes(v.launchpadView)?v.launchpadView:'pulls'};}catch{return defaults;}
}
export function validDefaultBranch(value:string){return value.length>0&&value.length<=200&&!value.startsWith('-')&&!value.startsWith('/')&&!value.endsWith('/')&&!value.endsWith('.')&&!/[\s~^:?*\[\\\x00-\x1f\x7f]/.test(value)&&!value.includes('..')&&!value.includes('@{')&&!value.includes('//')&&value!=='@'&&value.split('/').every(p=>!p.startsWith('.')&&!p.endsWith('.lock'));}
export function validInitialCommits(value:unknown):value is number{return typeof value==='number'&&Number.isInteger(value)&&value>=500&&value<=5000;}
export function saveGeneralPreferences(value:GeneralPreferences){if(!['pulls','issues','wips','snoozed','all','repositories'].includes(value.launchpadView))throw Error('Choose a valid Launchpad start view.');if(!validInitialCommits(value.initialCommits??500))throw Error('Initial commits must be an integer from 500 to 5000.');if(!validDefaultBranch(value.defaultBranch))throw Error('Enter a valid Git branch name.');localStorage.setItem(key,JSON.stringify(value));window.dispatchEvent(new Event('branchdesk-general-preferences'));}

export function useGeneralPreferences(){
 const [preferences,setPreferences]=useState(readGeneralPreferences);
 useEffect(()=>{const changed=()=>setPreferences(readGeneralPreferences());const external=(event:StorageEvent)=>{if(event.storageArea===localStorage&&(event.key===key||event.key===null))changed();};window.addEventListener('branchdesk-general-preferences',changed);window.addEventListener('storage',external);return()=>{window.removeEventListener('branchdesk-general-preferences',changed);window.removeEventListener('storage',external);};},[]);
 return preferences;
}
