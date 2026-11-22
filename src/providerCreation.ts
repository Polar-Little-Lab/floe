import {api} from './api';
import type {ProviderAction,ProviderReceipt} from './providerActions';
export type CreationMetadata={reviewers:string[];assignees:string[];labels:string[]};
export type CreationStep='reviewers'|'assignees'|'labels';
// Creation and follow-up writes are separate provider operations. Never recreate
// or retry after cancellation, failure, or an uncertain mutation response.
export async function createProviderItem(owner:string,repo:string,action:Extract<ProviderAction,{kind:'createPull'|'createIssue'}>,selection:CreationMetadata,onCreated:(receipt:ProviderReceipt)=>void,onApplied:(step:CreationStep)=>void){
 const receipt=await api.githubWrite(owner,repo,action);
 if(!receipt)return false;
 onCreated(receipt);
 const pull=action.kind==='createPull';
 const steps:CreationStep[]=(['reviewers','assignees','labels'] as const).filter(step=>(step!=='reviewers'||pull)&&selection[step].length>0);
 if(!steps.length)return true;
 const detail=await api.githubItemDetail(owner,repo,receipt.number,pull);
 if(!detail)return false;
 for(const step of steps){
  let change:ProviderAction;
  if(step==='reviewers'){
   if(!detail.reviewers)throw new Error('Requested reviewers were not returned. Inspect the created item before applying reviewers.');
   const added=selection.reviewers.filter(login=>!detail.reviewers!.some(existing=>existing.toLowerCase()===login.toLowerCase()));
   if(!added.length){onApplied(step);continue;}
   change={kind:'changeReviewers',number:receipt.number,reviewers:added,expectedReviewers:detail.reviewers,remove:false};
  }else if(step==='assignees'){
   if(!detail.assignees)throw new Error('Assignees were not returned. Inspect the created item before applying assignees.');
   const added=selection.assignees.filter(login=>!detail.assignees!.some(existing=>existing.toLowerCase()===login.toLowerCase()));
   if(!added.length){onApplied(step);continue;}
   change={kind:'setAssignees',number:receipt.number,pull,assignees:[...detail.assignees,...added],expectedAssignees:detail.assignees};
  }else{
   if(!detail.labels)throw new Error('Labels were not returned. Inspect the created item before applying labels.');
   const added=selection.labels.filter(label=>!detail.labels!.includes(label));
   if(!added.length){onApplied(step);continue;}
   change={kind:'setLabels',number:receipt.number,pull,labels:[...detail.labels,...added],expectedLabels:detail.labels};
  }
  if(!await api.githubWrite(owner,repo,change))return false;
  onApplied(step);
 }
 return true;
}
