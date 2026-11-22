import type {Action,Change,Snapshot} from './types';
export type StageSelection={snapshot:Snapshot;files:Change[];staged:boolean;invalidated?:boolean};
type Backend={mutate:(snapshot:Snapshot,action:Action)=>Promise<Snapshot>;refresh:(repoId:string)=>Promise<Snapshot>};

/** Never silently rebind reviewed file IDs to a newer revision or retry a mutation. */
export async function applyStageSelection(selection:StageSelection,backend:Backend,onRefresh:(snapshot:Snapshot)=>void,onInvalidated:()=>void):Promise<Snapshot>{
 if(selection.invalidated)throw 'STALE_STATE: Close this review and select files again from the current list.';
 try{
  return await backend.mutate(selection.snapshot,{kind:selection.staged?'unstageFiles':'stageFiles',file_ids:selection.files.map(file=>file.id)});
 }catch(error){
  if(!String(error).startsWith('CANCELLED:')){
   if(String(error).startsWith('STALE_STATE:'))onInvalidated();
   try{
    const latest=await backend.refresh(selection.snapshot.repoId);
    if(latest.repoId!==selection.snapshot.repoId){onInvalidated();throw new Error('Unexpected repository');}
    if(latest.revision!==selection.snapshot.revision)onInvalidated();
    onRefresh(latest);
   }catch{/* Keep the original mutation failure, even if refresh is unavailable. */}
  }
  throw error;
 }
}
