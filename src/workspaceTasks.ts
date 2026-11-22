import {repositoryScope,type ProviderPullPage,type ProviderPullRequest} from './providerPullRequests';
import type {ProviderIssue,ProviderIssuePage} from './providerIssues';
import type {NamedWorkspaceGroup} from './namedWorkspaces';
export type TaskKind='pulls'|'issues';
export type WorkspaceItem=ProviderPullRequest|ProviderIssue;
export interface TaskSource {key:string;repository:string;kind:TaskKind;items:WorkspaceItem[];loaded:boolean;loading:boolean;error:string;nextPage:number|null;retryPage:number;retryAppend:boolean}
export interface WorkspaceTaskState {sources:TaskSource[];running:boolean;cancelled:boolean}
type Reader=(owner:string,repo:string,page:number)=>Promise<ProviderPullPage|ProviderIssuePage>;
// Serialize provider requests across workspace switches, including the bounded
// in-flight request left behind by a disposed view. Cancellation skips queued work.
let providerQueue:Promise<unknown>=Promise.resolve();
export class WorkspaceTaskFeed {
 state:WorkspaceTaskState;private generation=0;private disposed=false;
 constructor(group:NamedWorkspaceGroup,private readers:Record<TaskKind,Reader>,private changed:(state:WorkspaceTaskState)=>void){
  const seen=new Set<string>();if(group.repositories.length>100)throw Error('Workspace repository limit exceeded.');
  this.state={running:false,cancelled:false,sources:group.repositories.flatMap(repo=>{
   repositoryScope(repo.owner+'/'+repo.repo);const repository=repo.owner+'/'+repo.repo,canonical=repository.toLowerCase();if(seen.has(canonical))throw Error('Duplicate workspace repository.');seen.add(canonical);
   return (['pulls','issues'] as const).map(kind=>({key:canonical+':'+kind,repository,kind,items:[],loaded:false,loading:false,error:'',nextPage:null,retryPage:1,retryAppend:false}));
  })};
 }
 private emit(next:WorkspaceTaskState){this.state=next;if(!this.disposed)this.changed(next);}
 private patch(key:string,patch:Partial<TaskSource>){this.emit({...this.state,sources:this.state.sources.map(s=>s.key===key?{...s,...patch}:s)});}
 cancel(){this.generation++;this.emit({...this.state,running:false,cancelled:true,sources:this.state.sources.map(s=>({...s,loading:false}))});}
 dispose(){this.disposed=true;this.cancel();}
 private async read(key:string,page:number,append:boolean,generation:number){
  const source=this.state.sources.find(s=>s.key===key);if(!source)return;
  this.patch(key,{loading:true,error:'',retryPage:page,retryAppend:append});
  try{
   const request=providerQueue.then(()=>{
    if(this.disposed||generation!==this.generation)return null;
    const [owner,repo]=repositoryScope(source.repository);return this.readers[source.kind](owner,repo,page);
   });providerQueue=request.catch(()=>undefined);
   const result=await request;if(this.disposed||generation!==this.generation||result===null)return;
   if(result.provider!=='github'||result.host!=='github.com'||result.repository!==source.repository||result.page!==page||(result.nextPage!==null&&result.nextPage!==page+1))throw Error('Provider returned a different repository or page.');
   const items=new Map((append?source.items:[]).map(item=>[item.id,item]));for(const item of result.items)items.set(item.id,item);
   this.patch(key,{items:[...items.values()],loaded:true,loading:false,nextPage:result.nextPage,error:''});
  }catch(e){if(!this.disposed&&generation===this.generation)this.patch(key,{loading:false,error:String(e).replace(/^Error: /,'')});}
 }
 async refresh(){if(this.disposed||this.state.running)return;const generation=++this.generation;this.emit({...this.state,running:true,cancelled:false});
  for(const source of this.state.sources){if(this.disposed||generation!==this.generation)return;await this.read(source.key,1,false,generation);}
  if(!this.disposed&&generation===this.generation)this.emit({...this.state,running:false});
 }
 async refreshSource(key:string){if(this.disposed||this.state.running||!this.state.sources.some(source=>source.key===key))return;
  const generation=++this.generation;this.emit({...this.state,running:true,cancelled:false});await this.read(key,1,false,generation);
  if(!this.disposed&&generation===this.generation)this.emit({...this.state,running:false});
 }
 async loadSource(key:string,more=false){if(this.disposed||this.state.running)return;const source=this.state.sources.find(s=>s.key===key);if(!source)return;
  const page=more?source.nextPage:source.error?source.retryPage:1,append=more||!!source.error&&source.retryAppend;if(page===null)return;
  const generation=++this.generation;this.emit({...this.state,running:true,cancelled:false});await this.read(key,page,append,generation);
  if(!this.disposed&&generation===this.generation)this.emit({...this.state,running:false});
 }
}
