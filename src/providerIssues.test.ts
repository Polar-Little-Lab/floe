import {it,expect} from 'vitest';
import {IssueFeed,filterIssues,type ProviderIssuePage,type ProviderIssue} from './providerIssues';
const item=(id='1'):ProviderIssue=>({id,number:1,title:'Fix [literal] 한글',author:null,assignees:['Alice'],labels:['bug'],milestone:{number:'2',title:'Release'},url:'https://github.com/o/r/issues/1'});
const page=(repository='o/r',n=1,items:ProviderIssue[]=[],nextPage:number|null=null):ProviderIssuePage=>({provider:'github',host:'github.com',repository,page:n,items,nextPage});
it('continues PR-only pages, retains retry page and deduplicates later rows',async()=>{
 let fail=true;const calls:number[]=[];const feed=new IssueFeed(async(o,r,p)=>{calls.push(p);if(p===2&&fail)throw Error('RATE_LIMIT');return page(o+'/'+r,p,p===1?[]:[item(),item()],p===1?2:null)},()=>{});
 await feed.load('bad/scope/path');expect(calls).toEqual([]);
 await feed.load('o/r');expect(feed.state.loaded).toBe(true);expect(feed.state.nextPage).toBe(2);expect(feed.state.items).toEqual([]);
 await feed.load('o/r',true);expect(feed.state.error).toBe('RATE_LIMIT');expect(feed.state.nextPage).toBe(2);
 fail=false;await feed.load('o/r',true);expect(calls).toEqual([1,2,2]);expect(feed.state.items).toEqual([item()]);expect(feed.state.nextPage).toBeNull();
});
it('rejects mismatched pages and ignores stale scope or disposed results',async()=>{
 const pending:((p:ProviderIssuePage)=>void)[]=[];let changes=0;const feed=new IssueFeed(()=>new Promise(resolve=>pending.push(resolve)),()=>changes++);
 const first=feed.load('o/old'),second=feed.load('o/new');pending[1](page('o/new',1,[item()]));await second;pending[0](page('o/old'));await first;expect(feed.state.repository).toBe('o/new');
 const mismatch=feed.load('o/r');pending[2](page('o/r',2));await mismatch;expect(feed.state.error).toContain('different');expect(feed.state.loaded).toBe(false);
 const last=feed.load('o/last');feed.dispose();const before=changes;pending[3](page('o/last'));await last;expect(changes).toBe(before);
});
it('searches loaded issue metadata literally with AND semantics',()=>{
 expect(filterIssues([item()],'alice BUG release 한글 [literal] #1')).toHaveLength(1);
 expect(filterIssues([item()],'.*')).toEqual([]);
 expect(filterIssues([{...item(),assignees:null,labels:null,milestone:null}],'alice')).toEqual([]);
});

it('intersects assignee, exact label and milestone identity without treating unknown as empty',()=>{
 const items=[item(),{...item('2'),assignees:['Bob'],labels:['Bug'],milestone:{number:'3',title:'Release'}},{...item('3'),assignees:null,labels:null,milestone:null}];
 expect(filterIssues(items,'',{assignee:'alice',label:'bug',milestone:'2'}).map(x=>x.id)).toEqual(['1']);
 expect(filterIssues(items,'',{label:'bug',milestone:'3'})).toEqual([]);
 expect(filterIssues(items,'Release',{milestone:'3'}).map(x=>x.id)).toEqual(['2']);
 expect(filterIssues(items,'',{})).toHaveLength(3);
 expect(filterIssues([{...item(),labels:['']}],'',{label:''})).toHaveLength(1);
});
