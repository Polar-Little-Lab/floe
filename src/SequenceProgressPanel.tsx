import {useIdentityFields} from './useIdentityFields';
import type {CommitIdentity} from './commitIdentity';
import {ResolutionEditor} from './MergePreviewDialog';
import {FileContentView} from './FileContentView';
import {useState} from 'react';
import type {Action,SequenceProgress,ReplayPlan,FileContent} from './types';
export function SequenceProgressPanel({defaultIdentity,progress,error,disabled,onAction,plan,content,onInspect,onRead}:{defaultIdentity?:CommitIdentity|null;progress?:SequenceProgress|null;error?:string|null;disabled:boolean;onAction:(action:Action)=>void;plan?:ReplayPlan|null;content?:FileContent|null;onInspect?:()=>void;onRead?:(id:string,conflict:number,side:'ancestor'|'ours'|'theirs')=>void}) {
 const [message,setMessage]=useState(''); const {author,email,setAuthor,setEmail}=useIdentityFields(defaultIdentity); const [allowEmpty,setAllowEmpty]=useState(false);
 if(!progress&&!error)return null;
 const complete=!!progress&&progress.completed===progress.total;
 return <section className="sequence-progress" aria-label="Sequence progress">
  {error?<p role="alert">{error} · Sequence state needs recovery; other writes are blocked.</p>:<p role="status">Replay sequence · {progress!.completed}/{progress!.total} steps completed{progress!.pending?' · Step awaiting completion':''} · other writes are blocked.</p>}
  <p>Stop &amp; keep commits는 이미 적용한 커밋과 현재 파일을 유지하고 진행 기록을 보관합니다. 원래 HEAD로 되돌리지 않습니다. 충돌 해결 내용은 저장 후 다음 단계 적용으로 반영됩니다. Abort sequence는 깨끗한 작업 트리에서 시작 HEAD와 파일로 되돌립니다.</p>
  {progress&&!progress.aborting&&!complete&&!progress.pending&&onInspect&&<button disabled={disabled} onClick={onInspect}>Inspect current step</button>}
  {plan&&progress&&!progress.aborting&&!progress.pending&&plan.target===progress.current?.oid&&<section aria-label="Sequence conflicts">{plan.blockers.filter(b=>b!=='CONFLICTS').map(b=><p key={b}>{b}</p>)}{plan.conflicts.map((c,i)=>{const id=(c.ours??c.theirs??c.ancestor)!.id;return <div className="merge-conflict" key={id}>{(['ancestor','ours','theirs'] as const).map(side=><p key={side}>{side}: {c[side]?.path??'Absent'}{c[side]&&onRead&&<button disabled={disabled} onClick={()=>onRead(c[side]!.id,i,side)}>Read {side}</button>}</p>)}<ResolutionEditor value={progress.resolutions?.[id]} busy={disabled} onSave={resolution=>onAction({kind:'resolveSequence',sequence_id:progress.id,ordinal:progress.completed,file_id:id,resolution})}/></div>;})}{plan.conflicts.length===0&&<p>No conflicts in this step.</p>}</section>}
  {content&&<FileContentView content={content}/>}
  {progress&&!progress.aborting&&!complete&&<form onSubmit={e=>{e.preventDefault();onAction({kind:'continueSequence',message,author,email,allow_empty:allowEmpty});}}>
   {!progress.pending&&<><input className="wide-input" aria-label="Sequence commit message" maxLength={10000} value={message} onChange={e=>setMessage(e.target.value)} placeholder="Message for the next commit" disabled={disabled}/><input className="wide-input" aria-label="Sequence committer name" maxLength={256} value={author} onChange={e=>setAuthor(e.target.value)} placeholder="Committer name" disabled={disabled}/><input className="wide-input" aria-label="Sequence committer email" maxLength={256} value={email} onChange={e=>setEmail(e.target.value)} placeholder="Committer email" disabled={disabled}/><label><input type="checkbox" checked={allowEmpty} onChange={e=>setAllowEmpty(e.target.checked)} disabled={disabled}/>Keep empty commit</label></>}
   {progress.pending&&<p>저장된 메시지·작성자·시각으로 준비된 단계를 재개합니다.</p>}
   <button className="primary" disabled={disabled||(!progress.pending&&(!message.trim()||!author.trim()||!email.trim()))}>{progress.pending?'Resume prepared step':'Apply next step'}</button>
  </form>}
  {progress&&!progress.aborting&&!complete&&<button disabled={disabled} onClick={()=>onAction({kind:'skipSequence',sequence_id:progress.id,ordinal:progress.completed})}>Skip current step</button>}
  {progress&&<button disabled={disabled} onClick={()=>onAction({kind:'abortSequence'})}>{progress.aborting?'Resume sequence abort':'Abort sequence to original HEAD'}</button>}
  {!progress?.aborting&&<button disabled={disabled} onClick={()=>onAction({kind:'stopSequence'})}>{complete?'Finish sequence':'Stop & keep commits'}</button>}
 </section>;
}
