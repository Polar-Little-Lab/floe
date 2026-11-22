import {useIdentityFields} from './useIdentityFields';
import type {CommitIdentity} from './commitIdentity';
import {FileContentView} from './FileContentView';
import {ResolutionEditor} from './MergePreviewDialog';
import {useState} from 'react';
import {X} from './OriginalIcons';
import {DiffView} from './DiffView';
import type {Commit,ReplayKind,ReplayPlan,ReplayDraft,MergeResolution,FileContent} from './types';
const blockers:Record<string,string>={DIRTY_WORKTREE:'작업 파일 또는 스테이징 영역에 변경이 있습니다.',DETACHED_HEAD:'브랜치를 체크아웃해야 합니다.',OPERATION_IN_PROGRESS:'다른 Git 작업이 진행 중입니다.',CONFLICTS:'충돌 해결이 필요합니다.'};
export function ReplayPreviewDialog({defaultIdentity,commit,plan,error,busy,onPreview,onReset,onClose,onApply,initialMessage,content,onRead,draft,onStart,onResolve,onContinue,onAbort,onSkip}:{defaultIdentity?:CommitIdentity|null;commit:Commit;plan:ReplayPlan|null;error:string;busy:boolean;onPreview:(kind:ReplayKind,mainline:number|null)=>void;onReset:()=>void;onClose:()=>void;initialMessage?:string;content?:FileContent|null;onRead?:(id:string,conflict:number,side:'ancestor'|'ours'|'theirs',kind:ReplayKind,mainline:number|null)=>void;draft?:ReplayDraft|null;onStart?:(kind:ReplayKind,mainline:number|null)=>void;onResolve?:(id:string,resolution:MergeResolution)=>void;onContinue?:(message:string,author:string,email:string,allowEmpty:boolean)=>void;onAbort?:()=>void;onSkip?:()=>void;onApply?:(kind:ReplayKind,mainline:number|null,message:string,author:string,email:string,allowEmpty:boolean)=>void}) {
 const [keepEmpty,setKeepEmpty]=useState(false);
 const [kind,setKind]=useState<ReplayKind>(draft?.kind??'cherryPick');
 const [parent,setParent]=useState(draft?.mainline?.toString()??'');
 const [message,setMessage]=useState(draft?.kind==='revert'?`Revert "${commit.summary}"\n\nThis reverts commit ${commit.oid}.`:initialMessage??commit.summary);
 const {author,email,setAuthor,setEmail}=useIdentityFields(defaultIdentity);
 const needsParent=commit.parents.length>1;
 const resolutionsReady=!!draft&&typeof plan?.resolvedEmpty==='boolean';
 return <div className="modal-backdrop"><section className="modal merge-preview replay-preview" role="dialog" aria-modal="true" aria-label="Cherry-pick / Revert preview">
  <div className="modal-header"><h2>Cherry-pick / Revert preview</h2><button autoFocus disabled={busy} aria-label="Close replay preview" data-modal-close onClick={onClose}><X size={18}/></button></div>
  <p>{commit.summary}</p><p className="history-base">{commit.oid}</p>
  <form className="replay-options" onSubmit={e=>{e.preventDefault();if(!busy&&(!needsParent||parent))onPreview(kind,needsParent?Number(parent):null);}}>
   <label>Operation<select aria-label="Replay operation" value={kind} disabled={busy||!!draft} onChange={e=>{setKind(e.target.value as ReplayKind);setMessage(e.target.value==='revert'?`Revert "${commit.summary}"\n\nThis reverts commit ${commit.oid}.`:initialMessage??commit.summary);setKeepEmpty(false);onReset();}}><option value="cherryPick">Cherry-pick</option><option value="revert">Revert</option></select></label>
   {needsParent&&<label>Mainline parent<select aria-label="Mainline parent" value={parent} disabled={busy||!!draft} onChange={e=>{setParent(e.target.value);setKeepEmpty(false);onReset();}}><option value="">Choose a parent</option>{commit.parents.map((oid,i)=><option key={i} value={i+1}>{i+1} · {oid.slice(0,12)}</option>)}</select></label>}
   <button type="submit" disabled={busy||(needsParent&&!parent)}>{busy?'Calculating…':'Preview changes'}</button>
  </form>
  <p>충돌 없는 결과는 바로 적용합니다. 충돌 해결 내용은 초안에 저장되고 Continue에서 반영됩니다. 해결 결과가 비어 있으면 Skip할 수 있습니다. 연속 커밋 처리는 구현 중입니다.</p>
  {plan&&!draft&&onStart&&plan.conflicts.length>0&&plan.blockers.every(b=>b==='CONFLICTS')&&<button disabled={busy} onClick={()=>onStart(kind,needsParent?Number(parent):null)}>Start conflict resolution</button>}
  {plan&&!draft&&plan.empty&&plan.conflicts.length===0&&plan.blockers.length===0&&<div role="status"><p>{plan.originallyEmpty?'선택한 커밋은 원래부터 파일 변경이 없습니다.':'선택한 작업의 결과가 현재 HEAD와 같습니다.'} 창을 닫으면 저장소는 바뀌지 않습니다.</p><label><input type="checkbox" aria-label="Keep empty replay commit" disabled={busy} checked={keepEmpty} onChange={e=>setKeepEmpty(e.target.checked)}/>Create an empty commit anyway</label></div>}
  {draft&&onAbort&&<button disabled={busy} onClick={onAbort}>Abort replay</button>}
  {draft&&plan?.resolvedEmpty&&onSkip&&<div role="status"><p>해결 결과가 현재 HEAD와 같습니다. Skip하면 커밋 없이 이 작업을 마칩니다.</p><button disabled={busy} onClick={onSkip}>Skip empty replay</button><label><input type="checkbox" aria-label="Keep empty replay commit" disabled={busy} checked={keepEmpty} onChange={e=>setKeepEmpty(e.target.checked)}/>Create an empty commit anyway</label></div>}
  {plan&&(!plan.resolvedEmpty||keepEmpty)&&plan.kind===kind&&plan.target===commit.oid&&(draft?onContinue&&plan.blockers.every(b=>b==='CONFLICTS')&&plan.conflicts.every(c=>!!draft.resolutions[(c.ours??c.theirs??c.ancestor)!.id]):onApply&&(!plan.empty||keepEmpty)&&plan.conflicts.length===0&&plan.blockers.length===0)&&<form className="replay-apply" aria-label="Apply replay" onSubmit={e=>{e.preventDefault();if(!busy&&message.trim()&&author.trim()&&email.trim()){if(draft)onContinue?.(message,author,email,!!plan.resolvedEmpty&&keepEmpty);else onApply?.(kind,needsParent?Number(parent):null,message,author,email,plan.empty&&keepEmpty);}}}>
   <label>Commit message<textarea aria-label="Replay message" disabled={busy} maxLength={10000} value={message} onChange={e=>setMessage(e.target.value)}/></label>
   <label>Committer name<input aria-label="Replay committer" disabled={busy} maxLength={256} value={author} onChange={e=>setAuthor(e.target.value)}/></label>
   <label>Committer email<input aria-label="Replay email" disabled={busy} maxLength={256} value={email} onChange={e=>setEmail(e.target.value)}/></label>
   <p>{kind==='cherryPick'?'원래 커밋의 작성자와 작성 시각을 보존합니다.':'입력한 identity를 새 커밋의 작성자와 committer로 사용합니다.'}</p>
   <button type="submit" disabled={busy||!message.trim()||!author.trim()||!email.trim()}>{draft?'Continue replay':`Apply ${kind==='cherryPick'?'cherry-pick':'revert'}`}</button>
  </form>}
  {error&&<p role="alert">{error}</p>}
  {plan&&<><h3>{resolutionsReady?'Saved conflict resolutions':plan.conflicts.length?'Conflicts':plan.empty?'No changes':'Changes ready to preview'}</h3><p className="history-base">HEAD {plan.head.slice(0,12)} · Parent {plan.parent?.slice(0,12)??'Empty tree'}</p>{plan.blockers.filter(b=>!(resolutionsReady&&b==='CONFLICTS')).map((b,i)=><p className="readonly-note" key={i}>{blockers[b]??b}</p>)}{plan.conflicts.map((c,i)=><div className="merge-conflict" key={i}>{(['ancestor','ours','theirs'] as const).map(side=><p key={side}><strong>{side==='ancestor'?'Base':side==='ours'?'Current':'Incoming'}: </strong>{c[side]?`${c[side]!.path} · ${c[side]!.oid.slice(0,12)}`:'Absent'}{c[side]&&onRead&&<button disabled={busy} onClick={()=>onRead(c[side]!.id,i,side,kind,needsParent?Number(parent):null)}>Read {side==='ancestor'?'Base':side==='ours'?'Current':'Incoming'}</button>}</p>)}{draft&&onResolve&&<ResolutionEditor value={draft.resolutions[(c.ours??c.theirs??c.ancestor)!.id]} busy={busy} onSave={resolution=>onResolve((c.ours??c.theirs??c.ancestor)!.id,resolution)}/>}</div>)}{content&&<section aria-label="Replay conflict version content"><h3>{content.path}</h3><FileContentView content={content}/></section>}{plan.conflicts.length===0?<DiffView diff={plan.diff} busy={false}/>:<p role="status">{plan.resolvedEmpty?'현재 HEAD와 같은 해결 결과입니다. Skip으로 커밋 없이 마칠 수 있습니다.':'충돌 파일은 위에서 해결 내용을 확인하세요. 저장한 내용은 Continue에서 적용됩니다.'}</p>}</>}
 </section></div>;
}
