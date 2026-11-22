import {describe,it,expect} from 'vitest';
import {DraftPersistence,type SavedDraft} from './draftPersistence';
import {restoreDraft,type TabDraft} from './tabDraft';
const draft=(message:string):TabDraft=>({message,description:'body',author:'',email:'',amend:true,headOid:'a'.repeat(40)});
describe('persisted commit drafts',()=>{
 it('serializes rapid edits with current revisions and restores independently after restart',async()=>{
  const disk=new Map<string,SavedDraft>();let writes=0;
  const api={load:async(id:string)=>disk.get(id)??{draft:null,revision:'missing',warning:null},save:async(id:string,revision:string,value:TabDraft|null)=>{expect(revision).toBe(disk.get(id)?.revision??'missing');await Promise.resolve();const state={draft:value,revision:String(++writes),warning:null};disk.set(id,state);return state;}};
  const store=new DraftPersistence(api);await store.load('a');await store.load('b');
  await Promise.all([store.save('a',draft('first')),store.save('a',draft('latest')),store.save('b',draft('other'))]);
  const restarted=new DraftPersistence(api);expect((await restarted.load('a'))?.message).toBe('latest');expect((await restarted.load('b'))?.message).toBe('other');
  expect(restoreDraft(await restarted.load('a'),'b'.repeat(40)).amend).toBe(false);
 });
 it('preserves newer external state and retries ordinary failures without advancing revision',async()=>{
  let fail=true;const revisions:string[]=[];const store=new DraftPersistence({load:async()=>({draft:draft('old'),revision:'r1',warning:null}),save:async(_id,revision,value)=>{revisions.push(revision);if(fail)throw Error('disk full');return {draft:value,revision:'r2',warning:null};}});
  await store.load('a');await expect(store.save('a',draft('new'))).rejects.toThrow('disk full');fail=false;await store.save('a',draft('new'));expect(revisions).toEqual(['r1','r1']);
 });
 it('refuses external revisions and only clears corruption through an explicit action',async()=>{
  let disk:SavedDraft={draft:draft('original'),revision:'1',warning:null};
  const store=new DraftPersistence({load:async()=>({...disk}),save:async(_id,revision,value)=>{if(revision!==disk.revision)throw Error('STALE');disk={draft:value,revision:'3',warning:null};return disk;}});
  await store.load('a');disk={draft:draft('external'),revision:'2',warning:null};
  await expect(store.save('a',draft('local'))).rejects.toThrow('STALE');expect(disk.draft?.message).toBe('external');
  await expect(store.clear('a')).rejects.toThrow('STALE');expect(disk.draft?.message).toBe('external');
  disk={draft:null,revision:'damaged',warning:'corrupt'};await expect(store.load('a')).rejects.toThrow('corrupt');await store.clear('a');expect(disk.draft).toBeNull();expect(disk.warning).toBeNull();
 });
 it('reloads external changes after refused writes, then saves against the new revision',async()=>{
  let disk:SavedDraft={draft:draft('original'),revision:'1',warning:null};let writes=0;
  const store=new DraftPersistence({load:async()=>({...disk}),save:async(_id,revision,value)=>{if(revision!==disk.revision)throw Error('STALE');writes++;disk={draft:value,revision:String(Number(disk.revision)+1),warning:null};return disk;}});
  await store.load('a');disk={draft:draft('external'),revision:'2',warning:null};
  await expect(store.save('a',draft('unsaved'))).rejects.toThrow('STALE');
  expect((await store.load('a'))?.message).toBe('external');expect(writes).toBe(0);
  await store.save('a',draft('continued'));expect(disk.draft?.message).toBe('continued');expect(writes).toBe(1);
 });
 it('queues reload behind in-flight writes and preserves the cache if read fails',async()=>{
  let disk:SavedDraft={draft:draft('old'),revision:'1',warning:null};let fail=false;
  let release:()=>void=()=>{};const wait=new Promise<void>(resolve=>{release=resolve;});
  const store=new DraftPersistence({load:async()=>{if(fail)throw Error('read failed');return {...disk};},save:async(_id,revision,value)=>{expect(revision).toBe(disk.revision);await wait;disk={draft:value,revision:'2',warning:null};return disk;}});
  await store.load('a');const writing=store.save('a',draft('latest'));const reading=store.load('a');release();await writing;expect((await reading)?.message).toBe('latest');
  fail=true;await expect(store.load('a')).rejects.toThrow('read failed');await store.save('a',draft('latest'));
  expect(disk.draft?.message).toBe('latest');
 });
 it('never overwrites unreadable or corrupt saved drafts automatically',async()=>{
  let writes=0;const store=new DraftPersistence({load:async()=>({draft:null,revision:'bad',warning:'corrupt'}),save:async()=>{writes++;throw Error('unexpected');}});
  await expect(store.load('a')).rejects.toThrow('corrupt');await expect(store.save('a',draft('new'))).rejects.toThrow('not replaced');await expect(store.save('unknown',draft('new'))).rejects.toThrow('not replaced');expect(writes).toBe(0);
 });
});
