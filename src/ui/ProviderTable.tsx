import {ActivityAge} from './ActivityAge';
import {createContext,useContext,useEffect,useState,type ReactNode} from 'react';
import {defaultLaunchpadTablePreferences} from '../launchpadViewPreferences';
export const ProviderTablePreferences=createContext(defaultLaunchpadTablePreferences);
export interface ProviderTableRow {id:string;updatedAt?:string|null;selected?:boolean;onOpen?:()=>void;status:ReactNode;item:ReactNode;author:ReactNode;collaborators:ReactNode;repository:ReactNode;actions:ReactNode}
export function ProviderTable({label,rows,empty}:{label:string;rows:ProviderTableRow[];empty?:ReactNode}){
 const [now,setNow]=useState(()=>Date.now());
 useEffect(()=>{const timer=window.setInterval(()=>setNow(Date.now()),60000);return()=>window.clearInterval(timer);},[]);
 const preferences=useContext(ProviderTablePreferences);
 const columns=[...(preferences.age!==false?[{key:'age' as const,title:'Updated'}]:[]),{key:'status' as const,title:'Status'},{key:'item' as const,title:'Item'},...(preferences.author?[{key:'author' as const,title:'Author'}]:[]),...(preferences.collaborators?[{key:'collaborators' as const,title:'Collaborators'}]:[]),...(preferences.repository?[{key:'repository' as const,title:'Repo/Branch'}]:[]),{key:'actions' as const,title:'Action'}];
 return <div className={`provider-table-scroll${preferences.compact?' compact':''}`}><table className="provider-table" aria-label={label}><thead><tr>{columns.map(column=><th key={column.key} scope="col">{column.title}</th>)}</tr></thead><tbody>{!rows.length&&<tr><td colSpan={columns.length}><div className="provider-empty-state">{empty??'No matching items.'}</div></td></tr>}{rows.map(row=><tr key={row.id} className={row.selected?'provider-row-selected':undefined} aria-current={row.selected?'true':undefined} onClick={row.onOpen?event=>{const target=event.target;if(target instanceof Element&&target.closest('button,a,input,select,textarea,summary,details,label'))return;if(window.getSelection()?.toString())return;row.onOpen?.();}:undefined}>{columns.map(column=><td key={column.key} data-column={column.key}>{column.key==='age'?<ActivityAge timestamp={row.updatedAt} now={now}/>:column.key==='actions'?<div className="provider-row-actions">{row.actions}</div>:row[column.key]}</td>)}</tr>)}</tbody></table></div>;
}
