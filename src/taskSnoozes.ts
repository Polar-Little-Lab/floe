import {repositoryScope} from './providerPullRequests';
export interface TaskIdentity {provider:'github';host:'github.com';account:'anonymous';repository:string;kind:'pulls'|'issues';id:string}
export interface TaskSnooze {task:TaskIdentity;until:number}
export function taskIdentity(repository:string,kind:'pulls'|'issues',id:string):TaskIdentity {
 repositoryScope(repository);if(!id||new TextEncoder().encode(id).length>256||/[\u0000-\u001f\u007f]/.test(id))throw Error('Invalid task identity.');
 return {provider:'github',host:'github.com',account:'anonymous',repository:repository.toLowerCase(),kind,id};
}
export function taskIdentityKey(task:TaskIdentity):string {return JSON.stringify([task.provider,task.host,task.account,task.repository.toLowerCase(),task.kind,task.id]);}
export function snoozeActive(entry:TaskSnooze,now:number):boolean {return Number.isSafeInteger(now)&&Number.isSafeInteger(entry.until)&&entry.until>now;}
// Expiry uses an absolute UTC instant. Clock rollback can extend apparent remaining time;
// never prune entries during a read or silently rewrite persistence.
export function snoozeTask(entries:TaskSnooze[],task:TaskIdentity,hours:1|4,now:number):TaskSnooze[]{
 if(!Number.isSafeInteger(now)||now<0||(hours!==1&&hours!==4)||!Number.isSafeInteger(now+hours*3600000))throw Error('Invalid snooze time.');
 const key=taskIdentityKey(task),retained=entries.filter(e=>snoozeActive(e,now)&&taskIdentityKey(e.task)!==key);
 if(retained.length>=1000)throw Error('Up to 1000 snoozed tasks are supported.');
 return [...retained,{task:{...task,repository:task.repository.toLowerCase()},until:now+hours*3600000}];
}
export function unsnoozeTask(entries:TaskSnooze[],task:TaskIdentity):TaskSnooze[]{const key=taskIdentityKey(task);return entries.filter(e=>taskIdentityKey(e.task)!==key);}

export function setTaskPinned(entries:TaskIdentity[],task:TaskIdentity,pinned:boolean):TaskIdentity[]{
 const key=taskIdentityKey(task),remaining=entries.filter(e=>taskIdentityKey(e)!==key);
 if(!pinned)return remaining;
 if(remaining.length>=1000)throw Error('Up to 1000 pinned tasks are supported.');
 return [...remaining,{...task,repository:task.repository.toLowerCase()}];
}
