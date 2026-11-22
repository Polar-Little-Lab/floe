import {useDraftPresence,type DraftPresence} from './useDraftPresence';
import {MarkdownEditor} from './ui/MarkdownEditor';
import {CopyTextButton} from './CopyTextButton';
import {useRef,useState} from 'react';
import {api} from './api';
import {repositoryScope} from './providerPullRequests';
import {appendDiscussion,type DiscussionEntry} from './providerDiscussionModel';
import {ProviderCommentCard} from './ProviderDiscussion';
type Props={onDirty?:DraftPresence;repository:string;number:number;pull:boolean;disabled:boolean;onBusy:(value:boolean)=>void;onSubmit:(body:string)=>Promise<boolean>;onEdit:(id:number,previous:string,body:string)=>Promise<boolean>;onDelete:(id:number,previous:string)=>Promise<boolean>};
export function ProviderConversation({repository,number,pull,disabled,onBusy,onSubmit,onEdit,onDelete,onDirty}:Props){
 const [draft,setDraft]=useState(''),[submitting,setSubmitting]=useState(false);
 const [editing,setEditing]=useState<{id:number;previous:string;body:string}|null>(null);
 const [entries,setEntries]=useState<DiscussionEntry[]>([]),[next,setNext]=useState<number|null>(null),[loaded,setLoaded]=useState(false),[limited,setLimited]=useState(false);
 const [busy,setBusy]=useState(false),[error,setError]=useState(''),[message,setMessage]=useState(''),[needsReload,setNeedsReload]=useState(false);
 const working=useRef(false),sending=useRef(false);const locked=disabled||busy||submitting;
 const remove=async(entry:DiscussionEntry)=>{
  if(locked||working.current||sending.current||editing)return;sending.current=true;setSubmitting(true);setError('');setMessage('');
  try{if(await onDelete(entry.id,entry.body)){setEntries(previous=>previous.filter(item=>item.id!==entry.id));setNeedsReload(true);setMessage('Comment deletion confirmed. Refresh comments before loading more pages.');}}
  catch(error){setError(String(error));}finally{sending.current=false;setSubmitting(false);}
 };
 useDraftPresence('conversation',!!draft||!!editing&&editing.body!==editing.previous,onDirty);
 const submit=async(edit:boolean)=>{
  const body=edit?editing?.body:draft;if(locked||working.current||sending.current||!body?.trim())return;
  sending.current=true;setSubmitting(true);setError('');setMessage('');
  try{const success=edit&&editing?await onEdit(editing.id,editing.previous,body):await onSubmit(body);
   if(success){if(edit&&editing){setEntries(previous=>previous.map(entry=>entry.id===editing.id?{...entry,body}:entry));setEditing(null);}else setDraft('');setNeedsReload(true);setMessage('Comment change confirmed. Refresh comments to load the latest conversation.');}
  }catch(error){setError(String(error));}finally{sending.current=false;setSubmitting(false);}
 };
 const load=async(page:number)=>{
  if(locked||working.current||sending.current||editing)return;working.current=true;setBusy(true);onBusy(true);setError('');setMessage('');
  try{const result=await api.githubItemComments(...repositoryScope(repository),number,page,pull);if(!result){setMessage('Comment loading cancelled.');return;}
   let combined:DiscussionEntry[];try{combined=appendDiscussion(page===1?[]:entries,result.entries);}catch(error){setNeedsReload(true);throw error;}
   setEntries(combined);setNext(result.nextPage);setLimited(result.limitReached);setLoaded(true);setNeedsReload(false);
  }catch(error){setError(String(error));}finally{working.current=false;setBusy(false);onBusy(false);}
 };
 return <section aria-label="General conversation comments">
  <div className="provider-discussion-heading"><h3>Conversation</h3><button disabled={locked||!!editing} onClick={()=>void load(1)}>{loaded?'Refresh comments':'Load comments'}</button></div>
  {busy&&<p role="status">Loading conversation…</p>}{message&&<p role="status">{message}</p>}{error&&<p role="alert">{error}{loaded?' Previous comments are retained.':''}</p>}
  {!loaded&&!busy&&<p className="history-base">Load general comments on this {pull?'pull request':'issue'}.</p>}{loaded&&!entries.length&&<p>No conversation comments returned.</p>}
  {entries.map(entry=><div key={entry.id}><ProviderCommentCard entry={entry}/>{editing?.id===entry.id?<form onSubmit={event=>{event.preventDefault();void submit(true);}}><MarkdownEditor label={`Edit comment ${entry.id}`} value={editing.body} onChange={body=>setEditing({...editing,body})} disabled={locked} rows={4}/><div className="setting-row"><button className="primary" disabled={locked||!editing.body.trim()||editing.body===editing.previous}>Save comment</button><button type="button" disabled={locked} onClick={()=>setEditing(null)}>Cancel edit</button><CopyTextButton text={editing.body} label="Copy edited comment"/></div><p className="history-base">GitHub checks edit permissions. The current remote body is checked before replacement.</p></form>:<details className="provider-more-actions"><summary aria-label={`Actions for comment ${entry.id}`}>More</summary><button disabled={locked||!!editing} onClick={()=>setEditing({id:entry.id,previous:entry.body,body:entry.body})}>Edit comment</button><button disabled={locked||!!editing} onClick={()=>void remove(entry)}>Delete comment…</button></details>}</div>)}
  {limited&&<p role="status">Display limit of 3,000 comments reached. More may exist on GitHub.</p>}{next!==null&&<button disabled={locked||needsReload||!!editing} onClick={()=>void load(next)}>Load more comments</button>}
  {loaded&&<p className="history-base">Refresh to see edits and new comments. Conversation pages are not a frozen snapshot.</p>}
  <form onSubmit={event=>{event.preventDefault();void submit(false);}}><MarkdownEditor label="New conversation comment" value={draft} onChange={setDraft} disabled={locked||!!editing} rows={4}/><div className="setting-row"><button className="primary" disabled={locked||!!editing||!draft.trim()}>{submitting?'Submitting…':'Comment'}</button><CopyTextButton text={draft} label="Copy comment draft" disabled={!draft}/></div><p className="history-base">Submission requires native confirmation. Cancelled or failed submissions keep your draft.</p></form>
 </section>;
}
