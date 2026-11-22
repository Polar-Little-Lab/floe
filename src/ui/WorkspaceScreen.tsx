import type {ReactNode} from 'react';
import {X} from '../OriginalIcons';
import './workspace-screen.css';
/** Full workspace surface under the native tab bar; nested dialogs stay above it. */
export function WorkspaceScreen({label,title,icon,sidebar,actions,children,busy=false,onClose,className='',modal=true,closeLabel}:{label:string;title?:string;icon?:ReactNode;sidebar?:ReactNode;actions?:ReactNode;children:ReactNode;busy?:boolean;onClose:()=>void;className?:string;modal?:boolean;closeLabel?:string}){
 return <section className={`workspace-screen ${className}`} role={modal?'dialog':'region'} aria-modal={modal?true:undefined} aria-label={label}>
  {sidebar&&<aside className="screen-sidebar">{sidebar}</aside>}
  <div className="screen-main">
   {title&&<header className="screen-heading"><h1>{icon}{title}</h1><div className="screen-actions">{actions}<button data-modal-close disabled={busy} aria-label={closeLabel??`Close ${label}`} onClick={onClose}><X size={20}/></button></div></header>}
   <div className="screen-content">{children}</div>
  </div>
 </section>;
}
export function ScreenNavigation<T extends string>({label,items,value,onChange,vertical=false,disabled=false}:{label:string;items:readonly {id:T;label:string;icon?:ReactNode;disabled?:boolean;unavailableReason?:string}[];value:T;onChange:(value:T)=>void;vertical?:boolean;disabled?:boolean}){
 return <nav className={`screen-navigation ${vertical?'vertical':'horizontal'}`} aria-label={label}>{items.map(item=><button key={item.id} disabled={disabled||item.disabled} title={item.unavailableReason} aria-label={item.unavailableReason?`${item.label}: ${item.unavailableReason}`:undefined} aria-pressed={value===item.id} aria-current={value===item.id?'page':undefined} onClick={()=>onChange(item.id)}>{item.icon}{item.label}</button>)}</nav>;
}
export function SettingsSection({title,children}:{title:string;children:ReactNode}){
 return <section className="settings-section" aria-label={title}><h3><span>{title}</span></h3>{children}</section>;
}
export function SettingField({label,children,hint}:{label:string;children:ReactNode;hint?:string}){
 return <div className="settings-field"><div className="settings-field-label">{label}</div><div className="settings-field-control">{children}{hint&&<p className="history-base">{hint}</p>}</div></div>;
}
