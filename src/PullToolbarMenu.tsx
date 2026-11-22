import {useRef,useState,type ReactNode} from 'react';
import {ChevronDown} from './OriginalIcons';
export type PullToolbarMode='pull'|'fetchAll'|'ff'|'ffOnly'|'rebase';
const key='branchdesk.pull-toolbar-mode.v1';
export function usePullToolbarMode(){
 const [mode,setMode]=useState<PullToolbarMode>(()=>{try{const value=localStorage.getItem(key);return value==='fetchAll'||value==='ff'||value==='ffOnly'||value==='rebase'?value:'pull';}catch{return 'pull';}});
 const [warning,setWarning]=useState('');
 const save=(next:PullToolbarMode)=>{setMode(next);try{localStorage.setItem(key,next);setWarning('');}catch{setWarning('Toolbar choice applies to this session only.');}};
 return {mode,save,warning};
}
export function PullToolbarMenu({children,mode,onMode,disabled,warning}:{children:ReactNode;mode:PullToolbarMode;onMode:(mode:PullToolbarMode)=>void;disabled:boolean;warning:string}){
 const menu=useRef<HTMLDetailsElement>(null);
 return <div className="pull-toolbar-control">{children}<details ref={menu} className="pull-toolbar-menu" onKeyDown={event=>{if(event.key==='Escape'){event.preventDefault();event.stopPropagation();if(menu.current){menu.current.open=false;menu.current.querySelector('summary')?.focus();}}}}><summary aria-label="Default pull or fetch operation"><ChevronDown size={12}/></summary><div><p>Select a default pull/fetch operation to execute when clicking this button.</p>{([{id:'fetchAll',label:'Fetch All'},{id:'ff',label:'Pull (fast-forward if possible)'},{id:'ffOnly',label:'Pull (fast-forward only)'},{id:'rebase',label:'Pull (rebase)'},{id:'pull',label:'Pull (repository settings)'}] as const).map(item=><label key={item.id}><input type="radio" name="pull-toolbar-mode" checked={mode===item.id} disabled={disabled} onChange={()=>{onMode(item.id);if(menu.current)menu.current.open=false;}}/>{item.label}</label>)}<p className="history-base">Repository settings uses the configured mode. Other choices override fast-forward/rebase policy for this Pull only. Git configuration is unchanged.</p>{warning&&<p role="status">{warning}</p>}</div></details></div>;
}
