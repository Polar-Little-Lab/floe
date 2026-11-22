import {useState} from 'react';
import {X} from './OriginalIcons';
import {DiffView} from './DiffView';
import type {Commit,ReplaySelection,ReplaySequencePlan} from './types';
export function ReplaySequenceDialog({initial,initialSelections,commits,plan,busy,error,onPreview,onReset,onClose,onStart,writable=false}:{initial:Commit;initialSelections?:ReplaySelection[];commits:Commit[];plan:ReplaySequencePlan|null;busy:boolean;error:string;onPreview:(selections:ReplaySelection[])=>void;onReset:()=>void;onClose:()=>void;onStart?:(selections:ReplaySelection[])=>void;writable?:boolean}) {
 const [items,setItems]=useState<ReplaySelection[]>(initialSelections?.length?initialSelections.slice(0,32):[{oid:initial.oid,kind:'cherryPick',mainline:null}]);
 const change=(next:ReplaySelection[])=>{setItems(next);onReset();};
 const replace=(index:number,value:Partial<ReplaySelection>)=>change(items.map((item,i)=>i===index?{...item,...value}:item));
 const move=(index:number,offset:number)=>{const next=[...items];[next[index],next[index+offset]]=[next[index+offset],next[index]];change(next);};
 const missingParent=items.some(item=>(commits.find(c=>c.oid===item.oid)?.parents.length??0)>1&&item.mainline===null);
 return <div className="modal-backdrop"><section className="modal merge-preview replay-preview replay-sequence" role="dialog" aria-modal="true" aria-label="Replay sequence preview">
  <div className="modal-header"><h2>Replay sequence preview</h2><button autoFocus disabled={busy} aria-label="Close sequence preview" data-modal-close onClick={onClose}><X size={18}/></button></div>
  <p>위에서 아래 순서로 이전 결과 위에 적용해 계산합니다. 첫 충돌에서 멈춥니다. 미리보기는 저장소를 변경하지 않습니다. Start sequence로 순서를 저장한 뒤 각 단계를 따로 실행합니다. 충돌은 해당 단계에서 저장한 해결 내용으로 처리합니다. 시작 후 Abort sequence로 원래 HEAD로 되돌릴 수 있습니다 (깨끗한 작업 트리 필요).</p>
  <div className="setting-row"><button disabled={busy||items.length<2} onClick={()=>change([...items].reverse())}>Reverse order</button><button disabled={busy} onClick={()=>change(items.map(item=>({...item,kind:'cherryPick'})))}>All cherry-pick</button><button disabled={busy} onClick={()=>change(items.map(item=>({...item,kind:'revert'})))}>All revert</button></div>
  <ol>{items.map((item,index)=>{const commit=commits.find(c=>c.oid===item.oid);return <li key={index} className="merge-conflict">
   <label>Commit {index+1}<select aria-label={`Sequence commit ${index+1}`} disabled={busy} value={item.oid} onChange={e=>replace(index,{oid:e.target.value,mainline:null})}>{commits.map(c=><option key={c.oid} value={c.oid}>{c.oid.slice(0,7)} · {c.summary}</option>)}</select></label>
   <label>Operation<select aria-label={`Sequence operation ${index+1}`} disabled={busy} value={item.kind} onChange={e=>replace(index,{kind:e.target.value as ReplaySelection['kind']})}><option value="cherryPick">Cherry-pick</option><option value="revert">Revert</option></select></label>
   {(commit?.parents.length??0)>1&&<label>Mainline parent<select aria-label={`Sequence mainline ${index+1}`} disabled={busy} value={item.mainline??''} onChange={e=>replace(index,{mainline:e.target.value?Number(e.target.value):null})}><option value="">Choose a parent</option>{commit!.parents.map((p,i)=><option key={i} value={i+1}>{i+1} · {p.slice(0,7)}</option>)}</select></label>}
   <button disabled={busy||index===0} onClick={()=>move(index,-1)} aria-label={`Move step ${index+1} up`}>Move up</button><button disabled={busy||index===items.length-1} onClick={()=>move(index,1)} aria-label={`Move step ${index+1} down`}>Move down</button><button disabled={busy||items.length===1} onClick={()=>change(items.filter((_,i)=>i!==index))} aria-label={`Remove step ${index+1}`}>Remove</button>
  </li>;})}</ol>
  <button disabled={busy||items.length>=32} onClick={()=>change([...items,{oid:initial.oid,kind:'cherryPick',mainline:null}])}>Add step</button>
  <button disabled={busy||missingParent} onClick={()=>onPreview(items)}>{busy?'Calculating…':'Preview sequence'}</button>
  {onStart&&<button disabled={busy||!writable||missingParent||!plan||plan.blockers.some(b=>b!=='CONFLICTS')} onClick={()=>onStart(items)}>Start sequence</button>}
  {error&&<p role="alert">{error}</p>}
  {plan&&<section aria-label="Sequence result"><p>HEAD {plan.head.slice(0,12)} · {plan.steps.length} / {plan.requested} steps calculated</p>{plan.blockers.map(b=><p key={b} className="readonly-note">{b}</p>)}{plan.stoppedAt!==null&&<p role="status">Stopped at step {plan.stoppedAt+1}. {plan.requested-plan.steps.length} later steps have not been calculated.</p>}{plan.steps.map((step,i)=><details key={i} open={!!step.conflicts.length}><summary>{i+1} · {step.selection.kind==='cherryPick'?'Cherry-pick':'Revert'} {step.selection.oid.slice(0,7)} · {step.conflicts.length?'Conflict':step.empty?'Empty result':'Changes'}</summary>{step.empty&&<p>{step.originallyEmpty?'Originally empty commit':'No additional changes at this step'}</p>}{step.conflicts.map((c,j)=><div className="merge-conflict" key={j}>{(['ancestor','ours','theirs'] as const).map(side=><p key={side}>{side}: {c[side]?`${c[side]!.path} · ${c[side]!.oid.slice(0,12)}`:'Absent'}</p>)}</div>)}{step.diff&&<DiffView diff={step.diff} busy={false}/>}</details>)}</section>}
 </section></div>;
}
