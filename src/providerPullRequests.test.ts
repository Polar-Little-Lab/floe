import {it,expect} from 'vitest';
import {PullRequestFeed,repositoryScope,filterPullRequests,type ProviderPullPage} from './providerPullRequests';
const item=(id:string,title=id)=>({id,number:Number(id),title,author:null,draft:null,headBranch:'feature',baseBranch:'main',requestedReviewers:[],url:'https://github.com/o/r/pull/'+id});
const page=(repository='o/r',n=1,ids=['1'],next:number|null=null):ProviderPullPage=>({provider:'github',host:'github.com',repository,page:n,items:ids.map(id=>item(id)),nextPage:next});
it('validates scope before request; failed page retries without losing loaded rows',async()=>{
 const calls:number[]=[];let fail=true;const feed=new PullRequestFeed(async(o,r,p)=>{calls.push(p);if(p===2&&fail)throw Error('RATE_LIMIT');return page(o+'/'+r,p,p===1?['1']:['1','2'],p===1?2:null)},()=>{});
 await feed.load('https://github.com/o/r');expect(calls).toEqual([]);expect(()=>repositoryScope('o/..')).toThrow();
 await feed.load('o/r');await feed.load('o/r',true);expect(feed.state.items.map(x=>x.id)).toEqual(['1']);expect(feed.state.nextPage).toBe(2);expect(feed.state.error).toBe('RATE_LIMIT');fail=false;await feed.load('o/r',true);expect(calls).toEqual([1,2,2]);expect(feed.state.items.map(x=>x.id)).toEqual(['1','2']);
});
it('late response and disposed feed never overwrite newer scope',async()=>{
 const pending:((p:ProviderPullPage)=>void)[]=[];const changes:string[]=[];const feed=new PullRequestFeed(()=>new Promise(resolve=>pending.push(resolve)),s=>changes.push(s.repository));
 const old=feed.load('o/old'),fresh=feed.load('o/new');pending[1](page('o/new'));await fresh;pending[0](page('o/old'));await old;expect(feed.state.repository).toBe('o/new');
 const last=feed.load('o/last');feed.dispose();const count=changes.length;pending[2](page('o/last'));await last;expect(changes.length).toBe(count);
});
it('mismatched response refuses data; empty is distinct from an error',async()=>{
 const feed=new PullRequestFeed(async()=>page('wrong/repo'),()=>{});await feed.load('o/r');expect(feed.state.loaded).toBe(false);expect(feed.state.items).toEqual([]);expect(feed.state.error).toContain('different');
 const empty=new PullRequestFeed(async()=>page('o/r',1,[]),()=>{});await empty.load('o/r');expect(empty.state.loaded).toBe(true);expect(empty.state.error).toBe('');
});

it('filters loaded text literally across fields without losing unknown draft state',()=>{
 const items=[{...item('1','Fix [parser] 한글'),author:'Alice',draft:true,requestedReviewers:['Bob']},{...item('2','Other'),author:'Bob',draft:false},{...item('3','Unknown')}];
 expect(filterPullRequests(items,'ALICE bob [parser]','all').map(x=>x.id)).toEqual(['1']);
 expect(filterPullRequests(items,'한글 #1','draft').map(x=>x.id)).toEqual(['1']);
 expect(filterPullRequests(items,'','not-draft').map(x=>x.id)).toEqual(['2']);
 expect(filterPullRequests(items,'feature main','unknown').map(x=>x.id)).toEqual(['3']);
 expect(filterPullRequests(items,'.*','all')).toEqual([]);
 expect(items.map(x=>x.id)).toEqual(['1','2','3']);
});

it('combines exact label/milestone IDs and retains no-metadata items only in unfiltered view',()=>{
 const items=[{...item('1'),labels:['bug',''],milestone:{number:'1',title:'Release'}},{...item('2'),labels:['Bug'],milestone:{number:'2',title:'Release'}},item('3')];
 expect(filterPullRequests(items,'','all','bug').map(x=>x.id)).toEqual(['1']);
 expect(filterPullRequests(items,'Release','all',null,'2').map(x=>x.id)).toEqual(['2']);
 expect(filterPullRequests(items,'','all','bug','2')).toEqual([]);
 expect(filterPullRequests(items,'','all','').map(x=>x.id)).toEqual(['1']);
 expect(filterPullRequests(items,'','all').length).toBe(3);
});
