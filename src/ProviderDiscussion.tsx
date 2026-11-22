import {DiscardDraftDialog} from './ui/DiscardDraftDialog';
import {useDraftPresence,type DraftPresence} from './useDraftPresence';
import {MarkdownEditor} from './ui/MarkdownEditor';
import {useUIPreferences,formatUIDate} from './uiPreferences';
import {MarkdownContent} from './ui/MarkdownContent';
import {useCallback,useMemo,useRef,useState} from 'react';
import {api} from './api';
import {repositoryScope} from './providerPullRequests';
import {appendDiscussion,discussionThreads,type DiscussionEntry} from './providerDiscussionModel';
import {CopyTextButton} from './CopyTextButton';
import {DiffView} from './DiffView';
import './provider-discussion.css';
type Props={onDirty?:DraftPresence;repository:string;number:number;head:string;base:string;disabled:boolean;onBusy:(busy:boolean)=>void;onStale:()=>void;onReply:(commentId:number,body:string)=>void|Promise<boolean>};
export function ProviderCommentCard({entry,head}:{entry:DiscussionEntry;head?:string}){
 const {preferences}=useUIPreferences();
 return <article className="provider-comment" aria-label={`Comment ${entry.id}`}>
  <header><strong>{entry.author??'Deleted account'}</strong><span>{entry.state?.replaceAll('_',' ')}</span><time>{entry.createdAt?formatUIDate(entry.createdAt,preferences):'Not submitted'}</time><CopyTextButton text={entry.url} label={`Copy comment ${entry.id} URL`}/></header>
  {head&&entry.commitSha&&entry.commitSha!==head&&<p className="history-base">Commented on an earlier commit: {entry.commitSha.slice(0,12)}</p>}
  <MarkdownContent text={entry.body||'No review message.'}/>
 </article>;
}
function ReplyEditor({commentId,disabled,onReply,onDirty}:{onDirty?:DraftPresence;commentId:number;disabled:boolean;onReply:(commentId:number,body:string)=>void|Promise<boolean>}){
 const [open,setOpen]=useState(false),[body,setBody]=useState(''),[sending,setSending]=useState(false),[error,setError]=useState(''),[message,setMessage]=useState('');
 const inFlight=useRef(false);
 useDraftPresence(`reply-${commentId}`,!!body,onDirty);
 const send=async()=>{
  if(disabled||inFlight.current||!body.trim())return;
  inFlight.current=true;setSending(true);setError('');setMessage('');
  try{const sent=await onReply(commentId,body);if(sent){setBody('');setOpen(false);setMessage('Reply sent. Refresh code comments to see it in the thread.');}else setMessage('Reply was not confirmed. Your draft is retained; check the item status before retrying.');}
  catch(error){setError(String(error));}
  finally{inFlight.current=false;setSending(false);}
 };
 return <div>{open?<form className="provider-comment" onSubmit={event=>{event.preventDefault();void send();}}>
  <MarkdownEditor label={`Reply to comment ${commentId}`} rows={3} disabled={disabled||sending} value={body} onChange={value=>{setBody(value);setError('');setMessage('');}}/>
  <div className="setting-row"><button type="button" disabled={disabled||sending} onClick={()=>setOpen(false)}>Cancel reply</button><button disabled={disabled||sending||!body.trim()}>{sending?'Sending…':'Send reply'}</button><CopyTextButton text={body} label={`Copy reply draft for comment ${commentId}`}/></div>
 </form>:<button disabled={disabled||sending} aria-label={`Reply to thread ${commentId}`} onClick={()=>setOpen(true)}>{body?'Continue reply':'Reply'}</button>}
 {error&&<p role="alert">{error} Your reply draft is retained.</p>}{message&&<p role="status">{message}</p>}</div>;
}

function DiscussionFeed({kind,...props}:Props&{kind:'reviews'|'comments'}){
 const {repository,number,head,base,disabled,onBusy,onStale,onReply,onDirty}=props;
 const [entries,setEntries]=useState<DiscussionEntry[]>([]),[next,setNext]=useState<number|null>(null),[limited,setLimited]=useState(false);
 const [loaded,setLoaded]=useState(false),[busy,setBusy]=useState(false),[error,setError]=useState(''),[message,setMessage]=useState(''),[needsReload,setNeedsReload]=useState(false);
 const working=useRef(false);
 const drafts=useRef<Record<string,boolean>>({});
 const [pendingPage,setPendingPage]=useState<number|null>(null),[draftVersion,setDraftVersion]=useState(0);
 const trackDraft=useCallback((id:string,dirty:boolean)=>{drafts.current[id]=dirty;onDirty?.(id,dirty);},[onDirty]);
 const requestLoad=(page:number)=>{if(disabled||working.current)return;if(Object.values(drafts.current).some(Boolean)){setPendingPage(page);return;}void load(page);};
 const name=kind==='reviews'?'reviews':'code comments';
 const threads=useMemo(()=>discussionThreads(entries),[entries]);
 async function load(page:number,discardDrafts=false){
  if(disabled||working.current)return;
  working.current=true;setBusy(true);onBusy(true);setError('');setMessage('');
  try{
   const result=await api.githubPullDiscussion(...repositoryScope(repository),number,page,head,base,kind);
   if(!result){setMessage('Discussion loading cancelled.');return;}
   let merged:DiscussionEntry[];
   try{merged=appendDiscussion(page===1?[]:entries,result.entries);}catch(e){setNeedsReload(true);setError(String(e));return;}
   if(discardDrafts){drafts.current={};setDraftVersion(value=>value+1);}
   setEntries(merged);setNext(result.nextPage);setLimited(result.limitReached);setLoaded(true);setNeedsReload(false);
  }catch(e){if(String(e).includes('PROVIDER_SNAPSHOT_CHANGED'))onStale();else setError(String(e));}
  finally{working.current=false;setBusy(false);onBusy(false);}
 }
 return <section aria-label={`Pull request ${name}`}>
  {pendingPage!==null&&<DiscardDraftDialog onCancel={()=>setPendingPage(null)} onDiscard={()=>{const page=pendingPage;setPendingPage(null);void load(page,true);}}/>}
  <div className="provider-discussion-heading"><h4>{kind==='reviews'?'Reviews':'Code comments'}</h4><button disabled={disabled||busy} onClick={()=>requestLoad(1)}>{loaded?`Refresh ${name}`:`Load ${name}`}</button></div>
  {busy&&<p role="status">Loading {name}…</p>}{message&&<p role="status">{message}</p>}{error&&<p role="alert">{error}</p>}
  {loaded&&<><p className="history-base">{entries.length} {name} loaded{next!==null?' · more available':''}. Refresh to see edits and new comments.</p>
   {!entries.length&&<p>No {name} returned.</p>}
   {limited&&<p role="status">This app displays up to 3,000 {name}. Additional entries may exist on GitHub.</p>}
   {kind==='reviews'?entries.map(entry=><ProviderCommentCard key={entry.id} entry={entry} head={head}/>):threads.map(thread=>{
    const first=thread.entries[0];
    return <details className="provider-comment-thread" key={thread.id} open><summary>{first.path} · {first.line!==null?`${first.side??''} line ${first.line}`:first.originalLine!==null?`original line ${first.originalLine}`:'file comment'} · {thread.entries.length} comments</summary>
     {thread.missingParent&&<p role="status">Earlier comment #{thread.id} is not loaded or is no longer available.</p>}
     {thread.invalidRelation&&<p role="status">Reply relationships could not be reconstructed for this comment.</p>}
     {first.diffHunk&&<details><summary>Show diff context</summary><DiffView diff={{text:first.diffHunk,truncated:true}} busy={disabled||busy} truncationNotice="Comment context is an excerpt, not the full file diff."/></details>}
     {thread.entries.map(entry=><ProviderCommentCard key={entry.id} entry={entry} head={head}/>)}
     {!thread.missingParent&&!thread.invalidRelation&&<ReplyEditor key={`${thread.id}:${draftVersion}`} onDirty={trackDraft} commentId={thread.id} disabled={disabled||busy} onReply={onReply}/>}
    </details>;
   })}
   {next!==null&&<button disabled={disabled||busy||needsReload} onClick={()=>requestLoad(next)}>Load more {name}</button>}
  </>}
 </section>;
}
export function ProviderDiscussion(props:Props){
 return <section aria-label="Pull request discussion"><h3>Comments</h3><DiscussionFeed kind="reviews" {...props}/><DiscussionFeed kind="comments" {...props}/><p className="history-base">Replies are submitted after native confirmation. Thread resolution state is not yet shown.</p></section>;
}
