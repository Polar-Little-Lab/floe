import {describe,it,expect} from 'vitest';
import {appendDiscussion,discussionThreads,type DiscussionEntry} from './providerDiscussionModel';
const entry=(id:number,replyTo:number|null=null):DiscussionEntry=>({id,replyTo,author:'a',body:'text',createdAt:null,commitSha:null,state:null,path:'same.ts',line:1,originalLine:1,side:'RIGHT',diffHunk:null,url:'https://github.com/o/r/pull/7'});
describe('review thread reconstruction',()=>{
 it('joins explicit replies even if the parent arrives on a later page',()=>{expect(discussionThreads([entry(2,1)])[0].missingParent).toBe(true);const groups=discussionThreads([entry(2,1),entry(1),entry(3,2)]);expect(groups).toHaveLength(1);expect(groups[0].entries.map(x=>x.id)).toEqual([1,2,3]);expect(groups[0].missingParent).toBe(false);});
 it('never merges unrelated comments sharing a path and line',()=>expect(discussionThreads([entry(1),entry(2)])).toHaveLength(2));
 it('retains every comment without looping on malformed cyclic replies',()=>{const groups=discussionThreads([entry(1,2),entry(2,1)]);expect(groups.flatMap(g=>g.entries).map(e=>e.id)).toEqual([1,2]);expect(groups.every(g=>g.invalidRelation)).toBe(true);});
 it('rejects overlapping page IDs before mutating previously displayed history',()=>{const old=[entry(1)];expect(()=>appendDiscussion(old,[entry(1)])).toThrow('Reload');expect(old).toHaveLength(1);expect(()=>appendDiscussion([],[entry(2),entry(2)])).toThrow();expect(appendDiscussion(old,[entry(2)])).toHaveLength(2);});
});
