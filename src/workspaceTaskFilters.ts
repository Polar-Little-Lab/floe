import type {TaskSource,WorkspaceItem} from './workspaceTasks';
export interface WorkspaceTaskFilters {author:string|null;label:string|null;milestone:string|null}
export const emptyTaskFilters:WorkspaceTaskFilters={author:null,label:null,milestone:null};
export function milestoneIdentity(repository:string,number:string){return JSON.stringify([repository.toLowerCase(),number]);}
export function filterWorkspaceTasks(source:TaskSource,query:string,filters:WorkspaceTaskFilters):WorkspaceItem[]{
 const terms=query.toLocaleLowerCase().trim().split(/\s+/).filter(Boolean);
 return source.items.filter(item=>{
  if(filters.author!==null&&item.author?.toLowerCase()!==filters.author.toLowerCase())return false;
  if(filters.label!==null&&!item.labels?.includes(filters.label))return false;
  if(filters.milestone!==null&&(!item.milestone||milestoneIdentity(source.repository,item.milestone.number)!==filters.milestone))return false;
  const text=[source.repository,source.kind,String(item.number),'#'+item.number,item.title,item.author??'',...(item.labels??[]),item.milestone?.title??'',...('headBranch'in item?[item.headBranch,item.baseBranch,...item.requestedReviewers]:item.assignees??[])].join('\n').toLocaleLowerCase();
  return terms.every(term=>text.includes(term));
 });
}
export function workspaceFilterOptions(sources:TaskSource[]){
 const authors=new Map<string,string>(),labels=new Set<string>(),milestones=new Map<string,string>();
 for(const source of sources)for(const item of source.items){
  if(item.author!==null)authors.set(item.author.toLowerCase(),item.author);
  for(const label of item.labels??[])labels.add(label);
  if(item.milestone)milestones.set(milestoneIdentity(source.repository,item.milestone.number),`${source.repository} · ${item.milestone.title} (#${item.milestone.number})`);
 }
 return {author:[...authors].sort().map(([value,title])=>({value,title})),label:[...labels].sort().map(value=>({value,title:value})),milestone:[...milestones].map(([value,title])=>({value,title}))};
}
