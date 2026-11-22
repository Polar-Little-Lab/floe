import {it,expect} from 'vitest';
import {filterWorkspaceTasks,workspaceFilterOptions,milestoneIdentity,emptyTaskFilters} from './workspaceTaskFilters';
import type {TaskSource} from './workspaceTasks';
const source=(repository:string):TaskSource=>({key:repository+':issues',repository,kind:'issues',loaded:true,loading:false,error:'',nextPage:null,retryPage:1,retryAppend:false,items:[{id:'1',number:1,title:'[literal] 한글',author:'Alice',assignees:['Bob'],labels:['bug',''],milestone:{number:'9007199254740993',title:'Release'},url:'https://github.com/'+repository+'/issues/1'}]});
it('milestone identity includes repository and lossless number, never title alone',()=>{
 const a=source('O/A'),b=source('o/b'),filter={...emptyTaskFilters,milestone:milestoneIdentity('o/a','9007199254740993')};expect(filterWorkspaceTasks(a,'',filter)).toHaveLength(1);expect(filterWorkspaceTasks(b,'',filter)).toHaveLength(0);expect(workspaceFilterOptions([a,b]).milestone).toHaveLength(2);
 expect(milestoneIdentity('O/A','1')).toBe(milestoneIdentity('o/a','1'));
});
it('intersects literal text,case-insensitive authors and exact labels; unknown never matches',()=>{
 const s=source('o/a');expect(filterWorkspaceTasks(s,'bob [literal] 한글',{...emptyTaskFilters,author:'alice',label:'bug'})).toHaveLength(1);expect(filterWorkspaceTasks(s,'',{...emptyTaskFilters,label:'Bug'})).toEqual([]);expect(filterWorkspaceTasks(s,'',{...emptyTaskFilters,label:''})).toHaveLength(1);s.items[0]={...s.items[0],author:null,labels:null,milestone:null};expect(filterWorkspaceTasks(s,'',emptyTaskFilters)).toHaveLength(1);expect(filterWorkspaceTasks(s,'',{...emptyTaskFilters,author:'alice'})).toEqual([]);
});
it('deduplicates options by actual identity while retaining different label case',()=>{
 const a=source('o/a'),b=source('o/a');b.items[0]={...b.items[0],author:'ALICE',labels:['Bug']};const options=workspaceFilterOptions([a,b]);expect(options.author).toHaveLength(1);expect(options.milestone).toHaveLength(1);expect(options.label.map(o=>o.value)).toEqual(['','Bug','bug']);
});
