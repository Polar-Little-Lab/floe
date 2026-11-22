export type CheckKind='checkRuns'|'commitStatuses';
export interface CheckEntry{id:number;name:string;status:string;conclusion:string|null;description:string|null;updatedAt:string|null;detailsUrl:string|null}
export interface CheckPage{entries:CheckEntry[];totalCount:number;nextPage:number|null;limitReached:boolean;combinedState:string|null}
export function checkPresentation(entry:CheckEntry,kind:CheckKind):{label:string;symbol:string;tone:string}{
 const result=kind==='commitStatuses'?entry.status:entry.status==='completed'?entry.conclusion:null;
 if(result==='success')return {label:'Success',symbol:'✓',tone:'added'};
 if(['failure','error','timed_out','action_required','startup_failure'].includes(result??''))return {label:result!.replaceAll('_',' '),symbol:'×',tone:'deleted'};
 if(['neutral','skipped','cancelled','stale'].includes(result??''))return {label:result!.replaceAll('_',' '),symbol:'–',tone:'history-base'};
 if((kind==='commitStatuses'&&entry.status==='pending')||(kind==='checkRuns'&&['queued','in_progress','waiting','requested','pending'].includes(entry.status)))return {label:entry.status.replaceAll('_',' '),symbol:'◷',tone:''};
 return {label:`Unknown result (${entry.status}${entry.conclusion?` / ${entry.conclusion}`:''})`,symbol:'?',tone:'history-base'};
}
export function appendChecks(previous:CheckPage|null,incoming:CheckPage):CheckPage{
 if(!previous)return incoming;
 if(previous.totalCount!==incoming.totalCount||previous.combinedState!==incoming.combinedState)throw Error('Build status changed between pages. Refresh this list.');
 const ids=new Set(previous.entries.map(entry=>entry.id));
 for(const entry of incoming.entries){if(ids.has(entry.id))throw Error('Build status changed between pages. Refresh this list.');ids.add(entry.id);}
 return {...incoming,entries:[...previous.entries,...incoming.entries]};
}
