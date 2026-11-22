import {providerWriteFailure} from './providerWriteFailure';
import {ChangeStats} from './ui/ChangeStats';
import {ProviderMergePanel} from './ProviderMergePanel';
import type {RegisterProviderNavigationGuard} from './useProviderPanel';
import {ProviderServiceStrip} from './ui/ProviderServiceStrip';
import {ProviderBranchPicker} from './ui/ProviderBranchPicker';
import {MetadataSelection} from './ui/MetadataSelection';
import {createProviderItem,type CreationMetadata,type CreationStep} from './providerCreation';
import {DiscardDraftDialog} from './ui/DiscardDraftDialog';
import {ProviderConversation} from './ProviderConversation';
import {ProviderMetadata} from './ProviderMetadata';
import {MarkdownEditor} from './ui/MarkdownEditor';
import {MarkdownContent} from './ui/MarkdownContent';
import {DetailWorkspaceScreen} from './ui/DetailWorkspaceScreen';
import {GitBranch} from './OriginalIcons';
import {ProviderChecks} from './ProviderChecks';
import {ProviderDiscussion} from './ProviderDiscussion';
import {ProviderPullFiles} from './ProviderPullFiles';
import {useCallback,useEffect,useRef,useState} from 'react';
import {api} from './api';
import {repositoryScope} from './providerPullRequests';
import type {ProviderAction,ProviderDetail,ProviderState} from './providerActions';
import {CopyTextButton} from './CopyTextButton';
// Reference: official create-pr-2025 and Launchpad Open Pull Request Panel captures.
// Keep existing modal/form classes; provider credentials stay in the native prompt.
type ProviderItemPanelProps={registerNavigationGuard?:RegisterProviderNavigationGuard;repository:string;pull:boolean;number?:number;onClose:(refreshList?:boolean)=>void};
export function ProviderItemPanel(props:ProviderItemPanelProps){return <ProviderItemContent key={JSON.stringify([props.repository.toLowerCase(),props.pull,props.number??'new'])} {...props}/>;}
function ProviderItemContent({repository,pull,number,onClose,registerNavigationGuard}:ProviderItemPanelProps){
 const [creationMetadata,setCreationMetadata]=useState<CreationMetadata>({reviewers:[],assignees:[],labels:[]});
 const [createdNumber,setCreatedNumber]=useState<number|null>(null),[openCreated,setOpenCreated]=useState(false);
 const [appliedMetadata,setAppliedMetadata]=useState<CreationStep[]>([]);
 const creationReceipt=useRef<number|null>(null);
 const [editing,setEditing]=useState(!number);
 const [section,setSection]=useState<'overview'|'files'|'checks'>('overview');
 const [reviewOpen,setReviewOpen]=useState(false);
 const [title,setTitle]=useState(''),[body,setBody]=useState(''),[head,setHead]=useState(''),[base,setBase]=useState('');
 const baseEdited=useRef(false);
 const [draft,setDraft]=useState(false),[state,setState]=useState<ProviderState>('open');
 const [detailVersion,setDetailVersion]=useState(0);
 const [detail,setDetail]=useState<ProviderDetail|null>(null),[comment,setComment]=useState('');
 const [event,setEvent]=useState<'COMMENT'|'APPROVE'|'REQUEST_CHANGES'>('COMMENT');
 const [method,setMethod]=useState<'merge'|'squash'|'rebase'>('merge');
 const [busy,setBusy]=useState(false),[message,setMessage]=useState(''),[error,setError]=useState('');
 const [finished,setFinished]=useState(false),[unknown,setUnknown]=useState(false),[url,setUrl]=useState('');
 const [stale,setStale]=useState(false),[discard,setDiscard]=useState<'close'|'refresh'|'navigate'|null>(null);
 const pendingNavigation=useRef<((refresh:boolean)=>void)|null>(null);
 const dirtyChildren=useRef<Record<string,boolean>>({});
 const onDirty=useCallback((id:string,dirty:boolean)=>{dirtyChildren.current[id]=dirty;},[]);
 const hasDraft=()=>Object.values(dirtyChildren.current).some(Boolean)||!!comment||(!finished&&(number?editing&&!!detail&&(title!==detail.title||body!==detail.body||state!==detail.state):!!title||!!body||!!head||baseEdited.current||draft||Object.values(creationMetadata).some(values=>values.length>0)));
 const inFlight=useRef(false);
 const needsListRefresh=useRef(false);
 const close=()=>{if(inFlight.current)return;if(hasDraft()){setDiscard('close');return;}onClose(needsListRefresh.current);};
 const refresh=()=>{if(inFlight.current||unknown)return;if(hasDraft()){setDiscard('refresh');return;}void load();};
 useEffect(()=>{
  if(!registerNavigationGuard||openCreated)return;
  registerNavigationGuard(proceed=>{
   if(inFlight.current)return;
   if(hasDraft()){pendingNavigation.current=proceed;setDiscard('navigate');return;}
   proceed(needsListRefresh.current);
  });
  return()=>registerNavigationGuard(null);
 });
 const locked=busy||finished||unknown||stale;
 const kind=pull?'Pull Request':'Issue';
 async function load(){
  if(inFlight.current||unknown)return;
  inFlight.current=true;setBusy(true);setError('');setMessage('');
  try{const result=await api.githubItemDetail(...repositoryScope(repository),number!,pull);
   if(result){setComment('');dirtyChildren.current={};setStale(false);setDetailVersion(value=>value+1);setDetail(result);setEditing(false);setSection('overview');setTitle(result.title);setBody(result.body);setState(result.state);setUrl(result.url);setFinished(false);}
   else setMessage('Read cancelled. Existing details and drafts were kept.');
  }catch(e){if(detail)setStale(true);setError(String(e));}finally{inFlight.current=false;setBusy(false);}
 }
 async function loadMetadataOptions(kind:'labels'|'assignees'|'reviewers'|'branches',page:number){
  if(inFlight.current||locked)return null;
  inFlight.current=true;setBusy(true);
  try{const result=await api.githubMetadataOptions(...repositoryScope(repository),kind,page);
   if(kind==='branches'&&page===1&&result?.defaultBranch&&!baseEdited.current&&!creationReceipt.current)setBase(result.defaultBranch);
   return result;
  }
  finally{inFlight.current=false;setBusy(false);}
 }
 async function saveReviewers(previous:string[],next:string[]){
  const removed=previous.filter(login=>!next.some(value=>value.toLowerCase()===login.toLowerCase()));
  const added=next.filter(login=>!previous.some(value=>value.toLowerCase()===login.toLowerCase()));
  if((removed.length&&added.length)||(!removed.length&&!added.length)){setError('Add or remove reviewers in separate saves. Case-only changes do not change GitHub accounts.');return false;}
  return write({kind:'changeReviewers',number:number!,reviewers:removed.length?removed:added,remove:removed.length>0,expectedReviewers:previous});
 }
 async function create(){
  if(inFlight.current||locked||creationReceipt.current)return;
  inFlight.current=true;setBusy(true);setError('');setMessage('');
  try{
   const completed=await createProviderItem(...repositoryScope(repository),pull?{kind:'createPull',title,body,head,base,draft}:{kind:'createIssue',title,body},creationMetadata,result=>{
    creationReceipt.current=result.number;setCreatedNumber(result.number);setUrl(result.url);needsListRefresh.current=true;setFinished(true);
   },step=>setAppliedMetadata(previous=>[...previous,step]));
   setMessage(completed?`${kind} #${creationReceipt.current}: creation and selected metadata confirmed by GitHub.`:creationReceipt.current?`${kind} #${creationReceipt.current} was created. Metadata application stopped; unapplied selections are listed below.`:'Cancelled. No item created.');
  }catch(e){const text=String(e);setError(text);if(creationReceipt.current)setMessage(`${kind} #${creationReceipt.current} was created. Metadata application stopped; inspect the item before applying remaining selections.`);if(text.includes('PROVIDER_WRITE_OUTCOME_UNKNOWN')){needsListRefresh.current=true;setUnknown(true);}}
  finally{inFlight.current=false;setBusy(false);}
 }
 async function write(action:ProviderAction){
  if(inFlight.current||locked)return false;
  inFlight.current=true;setBusy(true);setError('');setMessage('');
  try{const result=await api.githubWrite(...repositoryScope(repository),action);
   if(result){if(action.kind==='changeReviewers')setDetail(previous=>previous?{...previous,reviewers:action.remove?action.expectedReviewers.filter(login=>!action.reviewers.some(changed=>changed.toLowerCase()===login.toLowerCase())):[...action.expectedReviewers,...action.reviewers]}:previous);if(action.kind==='setAssignees')setDetail(previous=>previous?{...previous,assignees:action.assignees}:previous);if(action.kind==='setLabels')setDetail(previous=>previous?{...previous,labels:action.labels}:previous);if(action.kind==='setItemState'){setDetail(previous=>previous?{...previous,state:action.state}:previous);setState(action.state);}if(action.kind==='review')setComment('');needsListRefresh.current=true;setEditing(false);setFinished(action.kind!=='changeReviewers'&&action.kind!=='createComment'&&action.kind!=='updateComment'&&action.kind!=='deleteComment'&&action.kind!=='setItemState'&&action.kind!=='setLabels'&&action.kind!=='setAssignees');setUrl(result.url);setMessage(result.merged?'Pull request merged.':`${kind} #${result.number}: change confirmed by GitHub.`);return true;}
   else {setMessage('Cancelled. No change submitted.');return false;}
  }catch(e){const failure=providerWriteFailure(e);setError(failure.message);if(failure.refreshRequired){setStale(true);needsListRefresh.current=true;}if(failure.outcomeUnknown){needsListRefresh.current=true;setUnknown(true);}return false;}
  finally{inFlight.current=false;setBusy(false);}
 }
 // Defer until mount settles so StrictMode's setup/cleanup replay does not open two native reads.
 useEffect(()=>{if(!number)return;let active=true;queueMicrotask(()=>{if(active)void load();});return()=>{active=false;};},[]);
 const validTitle=!!title.trim();
 const content=<>
  {discard&&<DiscardDraftDialog onCancel={()=>{pendingNavigation.current=null;setDiscard(null);}} onDiscard={()=>{const action=discard;setDiscard(null);if(action==='navigate'){const proceed=pendingNavigation.current;pendingNavigation.current=null;proceed?.(needsListRefresh.current);}else if(action==='close')onClose(needsListRefresh.current);else{void load();}}}/>}
  {!number&&pull&&<ProviderServiceStrip/>}
  <p>GitHub · {repository}</p>
  {number&&<button disabled={busy||unknown} onClick={refresh}>{detail?'Refresh item':'Load item details'}</button>}
  {busy&&<p role="status">Waiting for native confirmation or GitHub…</p>}
  {message&&<p role="status">{message}</p>}{error&&<p role="alert">{error}</p>}
  {stale&&!unknown&&<p role="status">Refresh this item to load current GitHub state before submitting another change. Your drafts remain until you confirm a refresh.</p>}
  {unknown&&<p role="alert">GitHub may have applied this change. Submission is disabled in this panel. Inspect the item on GitHub before starting another operation.</p>}
  {needsListRefresh.current&&<p className="history-base">Closing this panel refreshes the list from GitHub. No write is retried.</p>}
  {createdNumber&&<section aria-label="Created item"><p>{kind} #{createdNumber} exists. Creation will not be repeated.</p>{(['reviewers','assignees','labels'] as const).filter(key=>creationMetadata[key].length>0).map(key=><p key={key}>{appliedMetadata.includes(key)?'Applied':'Not confirmed'} {key}: {creationMetadata[key].join(', ')}</p>)}<button disabled={busy} onClick={()=>setOpenCreated(true)}>Open created {kind}</button></section>}
  {url&&<CopyTextButton text={url} label="Copy GitHub item URL" disabled={busy}/>}
   {number&&detail&&<header className="provider-summary"><small>#{detail.number}</small><h2>{detail.title}</h2><span className={`provider-state ${detail.state}`}>{detail.merged?'Merged':detail.state==='closed'?'Closed':detail.draft?'Draft':'Open'}</span><p className="provider-branch-summary">{detail.author??'Unknown author'}{pull&&detail.headBranch&&detail.baseBranch?<>{' · '}<code>{detail.headBranch}</code> → <code>{detail.baseBranch}</code></>:null}</p><details className="provider-more-actions"><summary aria-label="Item actions">Actions</summary><button disabled={locked||editing||detail.merged===true} onClick={()=>void write({kind:'setItemState',number,pull,expectedState:detail.state,state:detail.state==='open'?'closed':'open'})}>{detail.state==='open'?`Close ${kind}`:`Reopen ${kind}`}</button><CopyTextButton text={detail.url} label="Copy item URL"/></details>{section!=='overview'&&<button disabled={busy} onClick={()=>setSection('overview')}>Back to overview</button>}</header>}
  {(!number||detail)&&<div className={number?"provider-detail-columns":undefined}><div className="provider-detail-main">

   {number&&detail&&section==='overview'&&!editing&&<section aria-label="Item description"><div className="provider-discussion-heading"><h3>Description</h3><button disabled={locked} onClick={()=>setEditing(true)}>Edit description</button></div><MarkdownContent text={detail.body||'No description provided.'}/></section>}
   <div hidden={number!==undefined&&(!editing||section!=='overview')}><form onSubmit={e=>{e.preventDefault();if(!validTitle||locked)return;if(number)void write({kind:pull?'updatePull':'updateIssue',number,title,body,state});else void create();}}>
    <fieldset disabled={locked}>
    {!number&&pull&&<><div className="setting-row"><label>From Repo<input value={repository} readOnly/></label><span>→</span><label>To Repo<input value={repository} readOnly/></label></div>
     <div className="provider-branch-pair"><ProviderBranchPicker label="Source branch" value={head} onChange={setHead} disabled={locked} onLoad={page=>loadMetadataOptions('branches',page)}/><ProviderBranchPicker label="Target branch" value={base} onChange={value=>{baseEdited.current=true;setBase(value);}} disabled={locked} onLoad={page=>loadMetadataOptions('branches',page)}/></div><p className="history-base">Push the source branch to GitHub before creating a pull request.</p></>}
    <label>Title<input className="wide-input" aria-label="Item title" value={title} maxLength={256} required onChange={e=>setTitle(e.target.value)}/></label>
    <MarkdownEditor label="Item description" value={body} onChange={setBody} disabled={locked}/>
    {!number&&<div className="provider-create-metadata">{(['reviewers','assignees','labels'] as const).filter(key=>pull||key!=='reviewers').map(key=><MetadataSelection key={key} title={key==='reviewers'?'Reviewers':key==='assignees'?'Assignees':'Labels'} values={creationMetadata[key]} disabled={locked} onChange={values=>setCreationMetadata(previous=>({...previous,[key]:values}))} onLoad={page=>loadMetadataOptions(key,page)}/>)}</div>}
    {!number&&pull&&<label><input type="checkbox" checked={draft} onChange={e=>setDraft(e.target.checked)}/>Submit as draft</label>}
    {number&&<label>State<select aria-label="Item state" value={state} onChange={e=>setState(e.target.value as ProviderState)}><option value="open">Open</option><option value="closed">Closed</option></select></label>}
    <div className="setting-row"><button type="button" onClick={()=>{if(number&&detail){setTitle(detail.title);setBody(detail.body);setState(detail.state);setEditing(false);}else close();}}>Cancel</button><button type="submit" disabled={!validTitle||(!number&&pull&&(!head||!base||head===base))}>{number?'Save changes':`Create ${kind}`}</button></div>
    </fieldset>
   </form></div>
   <div hidden={section!=='files'}>{number&&pull&&detail?.headSha&&detail.baseSha&&<ProviderPullFiles statistics={detail} active={section==='files'} key={detailVersion} repository={repository} number={number} head={detail.headSha} base={detail.baseSha} disabled={locked} onBusy={value=>{inFlight.current=value;setBusy(value);}} onStale={()=>{setStale(true);setError('Pull request changed while loading files. Refresh item details before reviewing or merging.');}}/>}</div>
   <div hidden={section!=='overview'}>{number&&detail&&<ProviderConversation onDirty={onDirty} onDelete={(commentId,expectedBody)=>write({kind:'deleteComment',number,pull,commentId,expectedBody})} onEdit={(commentId,expectedBody,body)=>write({kind:'updateComment',number,pull,commentId,expectedBody,body})} onSubmit={body=>write({kind:'createComment',number,pull,body})} key={`conversation-${detailVersion}`} repository={repository} number={number} pull={pull} disabled={locked} onBusy={value=>{inFlight.current=value;setBusy(value);}}/>}</div>
   <div hidden={section!=='overview'}>{number&&pull&&detail?.headSha&&detail.baseSha&&<ProviderDiscussion onDirty={onDirty} onReply={(commentId,body)=>write({kind:'replyReview',number,commentId,body})} key={`discussion-${detailVersion}`} repository={repository} number={number} head={detail.headSha} base={detail.baseSha} disabled={locked} onBusy={value=>{inFlight.current=value;setBusy(value);}} onStale={()=>{setStale(true);setError('Pull request changed while loading discussion. Refresh item details before reviewing or merging.');}}/>}</div>
   <div hidden={section!=='checks'}>{number&&pull&&detail?.headSha&&detail.baseSha&&<ProviderChecks active={section==='checks'} key={`checks-${detailVersion}`} repository={repository} number={number} head={detail.headSha} base={detail.baseSha} disabled={locked} onBusy={value=>{inFlight.current=value;setBusy(value);}} onStale={()=>{setStale(true);setError('Pull request changed while loading build status. Refresh item details before reviewing or merging.');}}/>}</div>
   {number&&pull&&detail?.headSha&&section==='overview'&&<ProviderMergePanel detail={detail} disabled={locked||editing} refreshDisabled={busy||unknown} method={method} onMethod={setMethod} onRefresh={refresh} onMerge={()=>void write({kind:'merge',number,sha:detail.headSha!,method})}/>}
   </div>
   {number&&pull&&detail?.headSha&&<section aria-label="Pull request review and metadata">
    <ChangeStats value={detail}/>
    <div className="provider-review-navigation"><button disabled={busy||editing||!detail.baseSha} onClick={()=>setSection('files')}>Review Code</button><button disabled={locked||editing||detail.state!=='open'} onClick={()=>{setSection('overview');setReviewOpen(true);}}>Submit a Review</button><button disabled={busy||editing||!detail.baseSha} onClick={()=>setSection('checks')}>Build Status</button></div>
    <p className="history-base">Reviewed commit: {detail.headSha}</p>
    <fieldset disabled={locked||detail.state!=='open'}>
     <div hidden={!reviewOpen}><h3>Review</h3><MarkdownEditor label="Review comment" value={comment} onChange={setComment} disabled={locked||detail.state!=='open'} rows={4}/>
     <select aria-label="Review action" value={event} onChange={e=>setEvent(e.target.value as typeof event)}><option value="COMMENT">Comment</option><option value="APPROVE">Approve</option><option value="REQUEST_CHANGES">Request changes</option></select>
     <button disabled={event!=='APPROVE'&&!comment.trim()} onClick={()=>void write({kind:'review',number,sha:detail.headSha!,body:comment,event})}>Submit review</button><button onClick={()=>setReviewOpen(false)}>Cancel review</button></div>

    </fieldset>
    <ProviderMetadata onLoadOptions={loadMetadataOptions} onReviewers={saveReviewers} onAssignees={(expectedAssignees,assignees)=>write({kind:'setAssignees',number,pull,expectedAssignees,assignees})} disabled={locked} onDirty={onDirty} onLabels={(expectedLabels,labels)=>write({kind:'setLabels',number,pull,expectedLabels,labels})} detail={detail} pull/>
   </section>}
   {number&&!pull&&detail&&<aside><ProviderMetadata onLoadOptions={loadMetadataOptions} onAssignees={(expectedAssignees,assignees)=>write({kind:'setAssignees',number,pull,expectedAssignees,assignees})} disabled={locked} onDirty={onDirty} onLabels={(expectedLabels,labels)=>write({kind:'setLabels',number,pull,expectedLabels,labels})} detail={detail} pull={false}/></aside>}
  </div>}
 </>;
 if(openCreated&&createdNumber)return <ProviderItemPanel registerNavigationGuard={registerNavigationGuard} repository={repository} pull={pull} number={createdNumber} onClose={()=>onClose(true)}/>;
 if(number)return <DetailWorkspaceScreen label={`${kind} #${number}`} title={`GitHub ${kind}`} icon={<GitBranch size={22}/>} busy={busy} onClose={close}>{content}</DetailWorkspaceScreen>;
 return <div className="modal-backdrop"><section className="modal worktrees-dialog provider-item-panel" role="dialog" aria-modal="true" aria-label={`Create ${kind}`}><header className="modal-header"><h2>Create {kind}</h2><button data-modal-close disabled={busy} onClick={close} aria-label="Close provider panel">×</button></header>{content}</section></div>;
}
