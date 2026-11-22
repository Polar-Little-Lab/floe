import {repositoryScope} from './providerPullRequests';
export interface ProviderIssue {id:string;number:number;title:string;updatedAt?:string|null;author:string|null;assignees:string[]|null;labels:string[]|null;milestone:{number:string;title:string}|null;url:string}
export interface ProviderIssuePage {provider:'github';host:'github.com';repository:string;page:number;nextPage:number|null;items:ProviderIssue[]}
export interface IssueState {repository:string;items:ProviderIssue[];nextPage:number|null;loading:boolean;error:string;loaded:boolean}
export class IssueFeed {
 state:IssueState={repository:'',items:[],nextPage:null,loading:false,error:'',loaded:false};
 private generation=0;
 constructor(private read:(owner:string,repo:string,page:number)=>Promise<ProviderIssuePage>,private changed:(state:IssueState)=>void){}
 private emit(next:IssueState){this.state=next;this.changed(next);}
 dispose(){this.generation++;}
 async load(input:string,append=false){
  if(append&&(this.state.loading||this.state.nextPage===null))return;
  let scope:[string,string];try{scope=repositoryScope(input);}catch(e){this.emit({...this.state,error:String(e).replace(/^Error: /,'')});return;}
  const repository=scope.join('/'),page=append?this.state.nextPage!:1;
  if(append&&repository!==this.state.repository)return;
  const generation=++this.generation;
  const previous=append||this.state.repository===repository?this.state:{repository,items:[],nextPage:null,loading:false,error:'',loaded:false};
  this.emit({...previous,loading:true,error:''});
  try{
   const result=await this.read(...scope,page);
   if(generation!==this.generation)return;
   if(result.provider!=='github'||result.host!=='github.com'||result.repository!==repository||result.page!==page||(result.nextPage!==null&&result.nextPage!==page+1))throw Error('Provider returned a different repository or page.');
   const items=new Map((append?previous.items:[]).map(item=>[item.id,item]));for(const item of result.items)items.set(item.id,item);
   this.emit({repository,items:[...items.values()],nextPage:result.nextPage,loading:false,error:'',loaded:true});
  }catch(e){if(generation===this.generation)this.emit({...previous,loading:false,error:String(e).replace(/^Error: /,'')});}
 }
}

export interface IssueFilters {assignee?:string|null;label?:string|null;milestone?:string|null}
export function filterIssues(items:ProviderIssue[],query:string,filters:IssueFilters={}):ProviderIssue[]{
 const terms=query.trim().toLocaleLowerCase().split(/\s+/).filter(Boolean);
 return items.filter(item=>{
  if(filters.assignee!=null&&!item.assignees?.some(value=>value.toLocaleLowerCase()===filters.assignee!.toLocaleLowerCase()))return false;
  if(filters.label!=null&&!item.labels?.includes(filters.label))return false;
  if(filters.milestone!=null&&item.milestone?.number!==filters.milestone)return false;
  const text=[String(item.number),'#'+item.number,item.title,item.author??'',...(item.assignees??[]),...(item.labels??[]),item.milestone?.title??''].join('\n').toLocaleLowerCase();return terms.every(term=>text.includes(term));});
}
