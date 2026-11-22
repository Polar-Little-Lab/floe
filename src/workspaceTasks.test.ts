import {it,expect} from 'vitest';
import {WorkspaceTaskFeed} from './workspaceTasks';
import type {NamedWorkspaceGroup} from './namedWorkspaces';
const group:NamedWorkspaceGroup={id:'g',name:'Team',repositories:[{owner:'o',repo:'a',localPath:null},{owner:'o',repo:'b',localPath:null}]};
const item=(title:string)=>({id:'same',number:1,title,author:null,assignees:null,labels:null,milestone:null,url:'https://github.com/o/a/issues/1'});
const page=(o:string,r:string,p:number,items=[item(r)],nextPage:number|null=null)=>({provider:'github' as const,host:'github.com' as const,repository:o+'/'+r,page:p,items,nextPage});
it('scopes identical IDs by repo/type, serializes and retains independent errors',async()=>{
 let active=0,max=0;const read=async(o:string,r:string,p:number)=>{active++;max=Math.max(max,active);await Promise.resolve();active--;if(r==='b')throw Error('UNAVAILABLE');return page(o,r,p)};
 const feed=new WorkspaceTaskFeed(group,{pulls:read,issues:read},()=>{});await feed.refresh();expect(max).toBe(1);expect(feed.state.sources.map(s=>s.items.length)).toEqual([1,1,0,0]);expect(feed.state.sources[2].error).toBe('UNAVAILABLE');expect(feed.state.running).toBe(false);
});
it('PR-only page continuation and failed next-page retry preserve prior source rows',async()=>{
 let fail=true;const calls:number[]=[];const read=async(o:string,r:string,p:number)=>{calls.push(p);if(p===2&&fail)throw Error('RATE_LIMIT');return page(o,r,p,p===1?[]:[item('later')],p===1?2:null)};
 const feed=new WorkspaceTaskFeed({...group,repositories:[group.repositories[0]]},{pulls:read,issues:read},()=>{});await feed.loadSource('o/a:issues');expect(feed.state.sources[1].nextPage).toBe(2);await feed.loadSource('o/a:issues',true);expect(feed.state.sources[1].retryPage).toBe(2);fail=false;await feed.loadSource('o/a:issues');expect(calls).toEqual([1,2,2]);expect(feed.state.sources[1].items[0].title).toBe('later');
});
it('refresh failure retains previous results and rejects a mismatched scope',async()=>{
 let wrong=false;const read=async(o:string,r:string,p:number)=>page(o,wrong?'wrong':r,p);const feed=new WorkspaceTaskFeed(group,{pulls:read,issues:read},()=>{});await feed.loadSource('o/a:issues');wrong=true;await feed.loadSource('o/a:issues');expect(feed.state.sources[1].items).toHaveLength(1);expect(feed.state.sources[1].error).toContain('different');
});
it('cancel skips remaining requests and ignores in-flight response',async()=>{
 let resolve!:(p:ReturnType<typeof page>)=>void,calls=0;const read=()=>{calls++;return new Promise<ReturnType<typeof page>>(r=>resolve=r)};const feed=new WorkspaceTaskFeed(group,{pulls:read,issues:read},()=>{});const run=feed.refresh();await Promise.resolve();expect(calls).toBe(1);feed.cancel();resolve(page('o','a',1));await run;expect(calls).toBe(1);expect(feed.state.cancelled).toBe(true);expect(feed.state.sources.every(s=>!s.loaded&&!s.loading)).toBe(true);
});
it('disposed old workspace cannot publish; new workspace waits for its request to settle',async()=>{
 let resolve!:(p:ReturnType<typeof page>)=>void,changes=0,newCalls=0;const oldRead=()=>new Promise<ReturnType<typeof page>>(r=>resolve=r);const old=new WorkspaceTaskFeed(group,{pulls:oldRead,issues:oldRead},()=>changes++);const running=old.refresh();await Promise.resolve();old.dispose();const before=changes;
 const freshRead=async(o:string,r:string,p:number)=>{newCalls++;return page(o,r,p)};const fresh=new WorkspaceTaskFeed(group,{pulls:freshRead,issues:freshRead},()=>{});const next=fresh.loadSource('o/b:issues');await Promise.resolve();expect(newCalls).toBe(0);resolve(page('o','a',1));await running;await next;expect(changes).toBe(before);expect(newCalls).toBe(1);expect(fresh.state.sources[3].items).toHaveLength(1);
});
it('rejects duplicate repository scope before any request',()=>{
 expect(()=>new WorkspaceTaskFeed({...group,repositories:[group.repositories[0],{owner:'O',repo:'A',localPath:null}]},{pulls:async()=>page('o','a',1),issues:async()=>page('o','a',1)},()=>{})).toThrow('Duplicate');
});
