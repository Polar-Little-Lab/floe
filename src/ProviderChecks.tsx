import {useInitialVisibleRead} from './useInitialVisibleRead';
import {useRef,useState} from 'react';
import {api} from './api';
import {repositoryScope} from './providerPullRequests';
import {appendChecks,checkPresentation,type CheckKind,type CheckPage} from './providerChecksModel';
import {CopyTextButton} from './CopyTextButton';
type Props={repository:string;number:number;head:string;base:string;disabled:boolean;onBusy:(busy:boolean)=>void;onStale:()=>void;active?:boolean};
function CheckFeed({kind,onLoaded,...props}:Props&{kind:CheckKind;onLoaded?:()=>void}){
 const {repository,number,head,base,disabled,onBusy,onStale,active=false}=props;
 const [data,setData]=useState<CheckPage|null>(null),[busy,setBusy]=useState(false),[error,setError]=useState(''),[message,setMessage]=useState(''),[needsRefresh,setNeedsRefresh]=useState(false),[observed,setObserved]=useState('');
 const working=useRef(false),name=kind==='checkRuns'?'check runs':'commit statuses';
 async function load(page:number){
  if(disabled||working.current)return;
  working.current=true;setBusy(true);onBusy(true);setError('');setMessage('');
  try{
   const result=await api.githubPullChecks(...repositoryScope(repository),number,page,head,base,kind);
   if(!result){setMessage('Build status loading cancelled.');return;}
   try{setData(appendChecks(page===1?null:data,result));}catch(e){setNeedsRefresh(true);setError(String(e));return;}
   setNeedsRefresh(false);setObserved(new Date().toLocaleTimeString());if(page===1)onLoaded?.();
  }catch(e){
   if(String(e).includes('PROVIDER_SNAPSHOT_CHANGED'))onStale();
   else{setError(String(e));if(String(e).includes('PROVIDER_CHECKS_CHANGED'))setNeedsRefresh(true);}
  }finally{working.current=false;setBusy(false);onBusy(false);}
 }
 useInitialVisibleRead(active,disabled||data!==null,()=>load(1));
 return <section aria-label={`Pull request ${name}`}>
  <div className="provider-discussion-heading"><h4>{kind==='checkRuns'?'Checks':'Commit statuses'}</h4><button disabled={disabled||busy} onClick={()=>void load(1)}>{data?`Refresh ${name}`:`Load ${name}`}</button></div>
  {busy&&<p role="status">Loading {name}…</p>}{error&&<p role="alert">{error}{data?' Previously loaded results remain below.':''}</p>}{message&&<p role="status">{message}</p>}
  {!data&&!busy&&<p className="history-base">Not loaded.</p>}
  {data&&<><p className="history-base">Last loaded at {observed} · {data.entries.length} of {data.totalCount} {name}.</p>
   {data.combinedState!==null&&<p>Last loaded combined commit status: {data.combinedState}. This does not include check runs.</p>}
   {!data.entries.length&&<p>No {name} reported. This does not mean checks passed.</p>}
   {data.limitReached&&<p role="status">This app displays up to 3,000 {name}; the list is incomplete.</p>}
   <ul className="provider-check-list">{data.entries.map(entry=>{const presentation=checkPresentation(entry,kind);return <li key={entry.id}><div><span className={presentation.tone} aria-label={presentation.label}>{presentation.symbol}</span> <strong>{entry.name}</strong> · {presentation.label}</div>{entry.description&&<p>{entry.description}</p>}{entry.updatedAt&&<p className="history-base">{entry.updatedAt}</p>}{entry.detailsUrl&&<CopyTextButton text={entry.detailsUrl} label={`Copy ${entry.name} build URL`} disabled={disabled||busy}/>}</li>;})}</ul>
   {data.nextPage!==null&&<button disabled={disabled||busy||needsRefresh} onClick={()=>void load(data.nextPage!)}>Load more {name}</button>}
  </>}
 </section>;
}
export function ProviderChecks(props:Props){const [checksLoaded,setChecksLoaded]=useState(false);return <section aria-label="Pull request build status"><h3>Build Status</h3><p className="history-base">Results for {props.head.slice(0,12)} in {props.repository}. Required checks and merge eligibility are not evaluated here.</p><CheckFeed kind="checkRuns" {...props} onLoaded={()=>setChecksLoaded(true)}/><CheckFeed kind="commitStatuses" {...props} active={!!props.active&&checksLoaded}/></section>;}
