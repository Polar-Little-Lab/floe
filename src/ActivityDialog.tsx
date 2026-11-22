import {useUIPreferences,formatUIDate} from './uiPreferences';
import {useCallback,useRef,useState} from 'react';
import {X} from './OriginalIcons';
export type Activity={id:number;time:number;kind:'notice'|'error';message:string};
export function useActivityFeedback(){
 const [notice,noticeState]=useState('샘플 미리보기 · 저장소를 열어 로컬 작업 시작');const [error,errorState]=useState('');const [events,setEvents]=useState<Activity[]>([]);const sequence=useRef(0);
 const record=useCallback((kind:Activity['kind'],message:string)=>{if(!message)return;const entry={id:++sequence.current,time:Date.now(),kind,message:message.slice(0,4000)};setEvents(previous=>[entry,...previous].slice(0,100));},[]);
 const setNotice=useCallback((message:string)=>{noticeState(message);record('notice',message);},[record]);
 const setError=useCallback((message:string)=>{errorState(message);record('error',message);},[record]);
 const clear=useCallback(()=>setEvents([]),[]);
 return {notice,setNotice,error,setError,events,clear};
}
export function ActivityMessages({events,onClear,showMessages=true}:{events:Activity[];onClear:()=>void;showMessages?:boolean}){
 const {preferences}=useUIPreferences();
 const [kind,setKind]=useState('all');const [query,setQuery]=useState('');
 const filtered=events.filter(event=>(showMessages||event.kind==='error')&&(!showMessages||kind==='all'||event.kind===kind)&&event.message.toLowerCase().includes(query.toLowerCase()));
 return <><p className="history-base">Recent messages from this session. Restarting the app clears this list.</p><div className="setting-row"><input className="wide-input" aria-label="Filter activity messages" placeholder="Search messages…" maxLength={200} value={query} onChange={event=>setQuery(event.target.value)}/><select aria-label="Activity type" value={!showMessages?'error':kind} disabled={!showMessages} onChange={event=>setKind(event.target.value)}><option value="all">All</option><option value="error">Errors</option><option value="notice">Messages</option></select><button disabled={!events.length} onClick={onClear}>Clear</button></div><div className="activity-events">{filtered.map(event=><article key={event.id} className={event.kind}><header><strong>{event.kind==='error'?'Error':'Message'}</strong><time dateTime={new Date(event.time).toISOString()}>{formatUIDate(event.time,preferences)}</time></header><p>{event.message}</p></article>)}{!filtered.length&&<p>No matching messages.</p>}</div></>;
}
export function ActivityDialog({events,onClose,onClear,showMessages=true,onSettings}:{events:Activity[];onClose:()=>void;onClear:()=>void;showMessages?:boolean;onSettings?:()=>void}){
 return <div className="modal-backdrop"><section className="modal activity-dialog" role="dialog" aria-modal="true" aria-label="Activity notifications"><div className="modal-header"><h2>Activity</h2>{onSettings&&<button onClick={onSettings}>Notification settings</button>}<button data-modal-close aria-label="Close activity" onClick={onClose}><X size={18}/></button></div><ActivityMessages events={events} onClear={onClear} showMessages={showMessages}/></section></div>;
}
