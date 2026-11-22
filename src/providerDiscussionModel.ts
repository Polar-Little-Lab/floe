export interface DiscussionEntry {
 id:number;author:string|null;body:string;createdAt:string|null;commitSha:string|null;
 state:string|null;replyTo:number|null;path:string|null;line:number|null;originalLine:number|null;
 side:string|null;diffHunk:string|null;url:string;
}
export interface DiscussionPage {entries:DiscussionEntry[];nextPage:number|null;limitReached:boolean}
export interface DiscussionThread {id:number;entries:DiscussionEntry[];missingParent:boolean;invalidRelation:boolean}
/** Group only explicit reply IDs, never infer a thread from a path/line match. */
export function discussionThreads(entries:DiscussionEntry[]):DiscussionThread[]{
 const byId=new Map(entries.map(entry=>[entry.id,entry])),groups=new Map<number,DiscussionThread>();
 for(const entry of entries){
  let current=entry,missingParent=false,invalidRelation=false,id=entry.id;
  const seen=new Set<number>();
  while(current.replyTo!==null){
   if(seen.has(current.id)||seen.size>=100){invalidRelation=true;id=entry.id;break;}
   seen.add(current.id);id=current.replyTo;
   const parent=byId.get(id);
   if(!parent){missingParent=true;break;}
   current=parent;
  }
  if(!missingParent&&!invalidRelation)id=current.id;
  const group=groups.get(id)??{id,entries:[],missingParent,invalidRelation};
  group.entries.push(entry);groups.set(id,group);
 }
 return [...groups.values()].map(group=>({...group,entries:[...group.entries.filter(entry=>entry.id===group.id),...group.entries.filter(entry=>entry.id!==group.id)]}));
}
export function appendDiscussion(previous:DiscussionEntry[],incoming:DiscussionEntry[]):DiscussionEntry[]{
 const ids=new Set(previous.map(entry=>entry.id));
 for(const entry of incoming){if(ids.has(entry.id))throw Error('Discussion changed between pages. Reload this list.');ids.add(entry.id);}
 return [...previous,...incoming];
}
