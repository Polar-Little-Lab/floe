import {it,expect} from 'vitest';
import {WorkspacePersistence,restoreWorkspace,orderRestoredTabs,type WorkspaceState} from './workspacePersistence';
it('restores in saved order through confirmed open; cancellation and missing paths preserve other tabs',async()=>{
 const calls:string[]=[];const result=await restoreWorkspace({paths:['one','cancel','missing','two'],active:'one'},async path=>{calls.push(path);if(path==='cancel')return null;if(path==='missing')throw Error('missing directory');return {id:path};});
 expect(calls).toEqual(['one','cancel','missing','two']);expect(result.opened.map(x=>x.value.id)).toEqual(['one','two']);expect(result.active).toEqual({id:'one'});expect(result.failures.map(x=>x.path)).toEqual(['cancel','missing']);
});
it('falls back to the last successfully reopened tab when the active path is unavailable',async()=>{
 const result=await restoreWorkspace({paths:['one','missing'],active:'missing'},async path=>path==='one'?path:null);expect(result.active).toBe('one');
});
it('serializes saves, preserves external workspace on stale revision, and reloads before retry',async()=>{
 let disk:WorkspaceState={workspace:null,revision:'1',warning:null};const revisions:string[]=[];
 const store=new WorkspacePersistence({load:async()=>({...disk}),save:async(revision,ids,active)=>{revisions.push(revision);if(revision!==disk.revision)throw Error('STALE');disk={workspace:{paths:ids,active},revision:String(Number(revision)+1),warning:null};return {...disk};}});
 await expect(store.save(['unknown'],null)).rejects.toThrow('not loaded');await store.load();
 await Promise.all([store.save(['a'],'a'),store.save(['a','b'],'b')]);expect(revisions).toEqual(['1','2']);
 disk={workspace:{paths:['external'],active:'external'},revision:'9',warning:null};await expect(store.save(['a'],'a')).rejects.toThrow('STALE');expect(disk.workspace?.paths).toEqual(['external']);
 await store.load();await store.save(['a'],'a');expect(disk.revision).toBe('10');
});

it('restores original order after partial retry while retaining unrelated tabs and aliased sessions',async()=>{
 const paths=['first','second','alias'];const ids=new Map<string,string>();let firstAttempt=true;
 const open=async(path:string)=>{if(path==='first'&&firstAttempt)return null;return {repoId:path==='alias'?'second':path};};
 const initial=await restoreWorkspace({paths,active:'first'},open);for(const item of initial.opened)ids.set(item.path,item.value.repoId);
 const previous=[{repoId:'unrelated'},{repoId:'second'}];firstAttempt=false;
 const retried=await restoreWorkspace({paths:initial.failures.map(x=>x.path),active:'first'},open);for(const item of retried.opened)ids.set(item.path,item.value.repoId);
 const tabs=[...previous,...retried.opened.map(x=>x.value)];
 expect(orderRestoredTabs(tabs,paths,ids).map(x=>x.repoId)).toEqual(['first','second','unrelated']);
 expect(tabs.map(x=>x.repoId)).toEqual(['unrelated','second','first']);expect(retried.active?.repoId).toBe('first');
});
