export interface ProviderPullRequest {id:string;number:number;title:string;updatedAt?:string|null;author:string|null;draft:boolean|null;headBranch:string;baseBranch:string;requestedReviewers:string[];labels?:string[]|null;milestone?:{number:string;title:string}|null;url:string}
export interface ProviderPullPage {provider:'github';host:'github.com';repository:string;page:number;nextPage:number|null;items:ProviderPullRequest[]}
export interface PullRequestState {repository:string;items:ProviderPullRequest[];nextPage:number|null;loading:boolean;error:string;loaded:boolean}
export function repositoryScope(input:string):[string,string]{
 const pieces=input.trim().split('/');
 if(pieces.length!==2||pieces.some(p=>!p||p.length>100||p==='.'||p==='..'||!/^[A-Za-z0-9_.-]+$/.test(p)))throw Error('Enter a GitHub repository as owner/repository.');
 return pieces as [string,string];
}
export class PullRequestFeed {
 state:PullRequestState={repository:'',items:[],nextPage:null,loading:false,error:'',loaded:false};
 private generation=0;
 constructor(private read:(owner:string,repo:string,page:number)=>Promise<ProviderPullPage>,private changed:(state:PullRequestState)=>void){}
 private emit(next:PullRequestState){this.state=next;this.changed(next);}
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

export type PullDraftFilter='all'|'draft'|'not-draft'|'unknown';
export function filterPullRequests(items:ProviderPullRequest[],query:string,draft:PullDraftFilter,label:string|null=null,milestone:string|null=null):ProviderPullRequest[]{
 const terms=query.trim().toLocaleLowerCase().split(/\s+/).filter(Boolean);
 return items.filter(item=>{
  if(draft==='draft'&&item.draft!==true||draft==='not-draft'&&item.draft!==false||draft==='unknown'&&item.draft!==null)return false;
  if(label!==null&&!item.labels?.includes(label)||milestone!==null&&item.milestone?.number!==milestone)return false;
  const text=[String(item.number),'#'+item.number,item.title,item.author??'',item.headBranch,item.baseBranch,...item.requestedReviewers,...(item.labels??[]),item.milestone?.title??''].join('\n').toLocaleLowerCase();
  return terms.every(term=>text.includes(term));
 });
}
