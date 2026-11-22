export interface Workspace {paths:string[];active:string|null}
export interface WorkspaceState {workspace:Workspace|null;revision:string;warning:string|null}
export class WorkspacePersistence {
 private state:WorkspaceState|undefined;
 private queue:Promise<unknown>=Promise.resolve();
 constructor(private api:{load:()=>Promise<WorkspaceState>;save:(revision:string,ids:string[],active:string|null)=>Promise<WorkspaceState>}){}
 load():Promise<WorkspaceState>{return this.enqueue(async()=>this.state=await this.api.load());}
 private enqueue<T>(task:()=>Promise<T>):Promise<T>{const result=this.queue.catch(()=>{}).then(task);this.queue=result;return result;}
 save(ids:string[],active:string|null,shouldSave:()=>boolean=()=>true):Promise<void>{return this.enqueue(async()=>{
  if(!shouldSave())return;
  if(!this.state)throw Error('Saved tabs were not loaded; the previous workspace was preserved.');
  this.state=await this.api.save(this.state.revision,ids,active);
 });}
}
// Each path goes through the same trust-confirming open command as a manual open.
// A missing/cancelled repository never prevents other saved tabs from opening.
export async function restoreWorkspace<T>(workspace:Workspace,open:(path:string)=>Promise<T|null>){
 const opened:{path:string;value:T}[]=[];const failures:{path:string;error:string}[]=[];
 for(const path of workspace.paths){try{const value=await open(path);if(value)opened.push({path,value});else failures.push({path,error:'Opening cancelled'});}catch(e){failures.push({path,error:String(e)});}}
 return {opened,failures,active:opened.find(item=>item.path===workspace.active)?.value??opened.at(-1)?.value};
}

// A failed earlier tab can succeed on a later retry. Restore original order,
// keeping unrelated tabs afterward and retaining each session exactly once.
export function orderRestoredTabs<T extends {repoId:string}>(tabs:T[],paths:string[],ids:Map<string,string>):T[]{
 const rank=new Map<string,number>();
 for(const path of paths){const id=ids.get(path);if(id&&!rank.has(id))rank.set(id,rank.size);}
 return [...tabs].sort((a,b)=>(rank.get(a.repoId)??rank.size)-(rank.get(b.repoId)??rank.size));
}
