import type {TabDraft} from './tabDraft';
export interface SavedDraft {draft:TabDraft|null;revision:string;warning:string|null}
export interface DraftApi {load:(id:string)=>Promise<SavedDraft>;save:(id:string,revision:string,draft:TabDraft|null)=>Promise<SavedDraft>}
// Serialize per-session writes so an old keystroke cannot overwrite a newer one.
export class DraftPersistence {
 private states=new Map<string,SavedDraft>();
 private queues=new Map<string,Promise<unknown>>();
 constructor(private api:DraftApi){}
 private enqueue<T>(id:string,task:()=>Promise<T>):Promise<T>{
  const result=(this.queues.get(id)??Promise.resolve()).catch(()=>{}).then(task);
  this.queues.set(id,result);return result;
 }
 load(id:string):Promise<TabDraft|undefined>{return this.enqueue(id,async()=>{
  const state=await this.api.load(id);this.states.set(id,state);
  if(state.warning)throw new Error(state.warning);
  return state.draft??undefined;
 });}
 clear(id:string):Promise<void>{return this.enqueue(id,async()=>{
  const state=this.states.get(id);if(!state)throw new Error('Draft could not be loaded; saved data was not replaced.');
  this.states.set(id,await this.api.save(id,state.revision,null));
 });}
 save(id:string,draft:TabDraft):Promise<void>{return this.enqueue(id,async()=>{
  const state=this.states.get(id);
  if(!state||state.warning)throw new Error('Draft could not be loaded; existing saved data was not replaced.');
  if(JSON.stringify(state.draft)===JSON.stringify(draft))return;
  this.states.set(id,await this.api.save(id,state.revision,{...draft}));
 });}
}
