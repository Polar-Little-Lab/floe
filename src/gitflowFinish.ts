import {api, type GitflowFinishPreview} from './api';
import type {Action, MergePlan, Snapshot} from './types';
import type {CommitIdentity} from './commitIdentity';

export type GitflowFinishOptions={identity:CommitIdentity;tagMessage:string;deleteBranch:boolean;reuseTag:boolean};
export type GitflowFinishProgress={active:string;completed:string[];state:'running'|'paused'|'failed'|'complete'};
export type GitflowFinishResult={snapshot:Snapshot;completed:string[];merge?:MergePlan};

// Each native mutation retains its own confirmation, revision check and recovery journal.
// A failed sequence is deliberately not rolled back: completed merges remain visible.
export async function finishGitflow(initial:Snapshot,preview:GitflowFinishPreview,options:GitflowFinishOptions,onSnapshot:(snapshot:Snapshot)=>void,onProgress?:(progress:GitflowFinishProgress)=>void):Promise<GitflowFinishResult>{
 let snapshot=initial;
 const completed:string[]=[];
 const report=(active:string,state:GitflowFinishProgress['state']='running')=>onProgress?.({active,completed:[...completed],state});
 const mutate=async(action:Action,label:string)=>{
  report(label);
  snapshot=await api.mutate(snapshot,action);
  completed.push(label);
  onSnapshot(snapshot);
  report(label);
 };
 try{
  report('Checking Gitflow preview');
  if(initial.revision!==preview.revision)throw Error('Gitflow preview expired. Reload before finishing.');
  if(!options.identity.name.trim()||!options.identity.email.includes('@'))throw Error('A commit author name and email are required.');
  if(preview.tag&&!preview.tagExists&&!options.tagMessage.trim())throw Error('Enter a tag message.');
  if(initial.changes.length||initial.writeBlock||initial.recoveryPending||initial.mergeDraft||initial.externalMerge||initial.rebaseDraft||initial.replayDraft||initial.sequenceProgress)throw Error('Finish requires a clean, writable repository.');
  const fresh=await api.gitflowFinishPreview(snapshot,preview.kind,preview.source);
  if(JSON.stringify(fresh)!==JSON.stringify(preview))throw Error('Gitflow source, targets or configuration changed. Reload the preview.');
  if(fresh.tagExists&&(!fresh.reusableTag||!options.reuseTag))throw Error('The version tag already exists. Verify and explicitly retain the matching tag before continuing.');
  let expectedTagObject=fresh.tagObjectOid;
  const {name:author,email}=options.identity;
  for(let index=0;index<fresh.targets.length;index++){
   const target=fresh.targets[index];
   if(snapshot.branch!==target.branch||snapshot.detached)await mutate({kind:'checkout',name:target.branch},`Checked out ${target.branch}`);
   report(`Inspecting merge into ${target.branch}`);
   const merge=await api.mergePreview(snapshot,fresh.source);
   if(merge.targetOid!==fresh.sourceOid||merge.headOid!==target.oid)throw Error('A Gitflow branch changed during Finish. Reload and inspect completed steps.');
   if(merge.kind==='conflicts'){report(`Resolve conflicts in ${target.branch}, then reopen Finish for ${fresh.source}.`,'paused');return {snapshot,completed,merge};}
   if(merge.blockers.length)throw Error(merge.blockers.join('\n'));
   if(merge.kind==='fastForward')await mutate({kind:'fastForwardMerge',name:fresh.source},`Merged into ${target.branch}`);
   else if(merge.kind==='mergeCommit')await mutate({kind:'mergeCommit',name:fresh.source,message:`Merge ${fresh.source} into ${target.branch}`,author,email},`Merged into ${target.branch}`);
   else if(merge.kind!=='upToDate')throw Error(`Cannot finish this merge automatically: ${merge.kind}`);
   if(index===0&&fresh.tag&&!fresh.tagExists){
    if(!snapshot.headOid)throw Error('Missing merged commit for the version tag.');
    await mutate({kind:'createAnnotatedTag',name:fresh.tag,oid:snapshot.headOid,message:options.tagMessage,author,email},`Created tag ${fresh.tag}`);
    const createdTag=await api.tagDetail(snapshot,fresh.tag);
    if(!createdTag.annotated||createdTag.commitOid!==snapshot.headOid)throw Error('Created tag could not be verified. Branch retained.');
    expectedTagObject=createdTag.objectOid;
   }
  }
  // Re-read ancestry and source identity before deleting; no force deletion.
  report('Verifying all targets and version tag');
  const final=await api.gitflowFinishPreview(snapshot,fresh.kind,fresh.source);
  if(final.sourceOid!==fresh.sourceOid||!final.targets.every(target=>target.alreadyContainsSource))throw Error('Source is not contained in every Gitflow target. Branch retained.');
  if(final.tag!==fresh.tag||final.targets.map(target=>target.branch).join('\n')!==fresh.targets.map(target=>target.branch).join('\n'))throw Error('Gitflow configuration changed during Finish. Branch retained.');
  if(fresh.tag&&(!final.reusableTag||!expectedTagObject||final.tagObjectOid!==expectedTagObject))throw Error('The version tag changed during Finish. Branch retained.');
  if(options.deleteBranch)await mutate({kind:'deleteBranch',name:fresh.source},`Deleted ${fresh.source}`);
  report(`Finished ${fresh.source}`,'complete');
  return {snapshot,completed};
 }catch(error){
  report(String(error),'failed');
  try{onSnapshot(await api.refresh(initial.repoId));}catch{/* Preserve original error; do not retry writes. */}
  throw Error(`${String(error)}\nConfirmed steps: ${completed.length?completed.join('; '):'none'}. Reload before retrying; completed changes are retained.`);
 }
}
