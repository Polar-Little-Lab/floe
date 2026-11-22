import { describe, expect, it } from 'vitest';
import { buildGraph } from './graph';
import type { Commit } from './types';
const c=(oid:string,parents:string[]):Commit=>({oid,parents,summary:oid,author:'Test',time:0,refs:[]});
describe('Persistent graph lanes',()=>{
 it('connects a linear history',()=>{const r=buildGraph([c('a',['b']),c('b',[])]);expect(r[0].edges).toEqual([{from:0,to:0,color:0}]);expect(r[1].incoming).toEqual([{from:0,to:0,color:0}]);});
 it('keeps parallel branch lines until the actual shared parent',()=>{const r=buildGraph([c('a',['r']),c('b',['r']),c('c',['r']),c('r',[])]);expect(r.map(x=>x.lane)).toEqual([0,1,2,0]);expect(r[2].edges).toHaveLength(3);expect(r[3].incoming.map(x=>x.to)).toEqual([0,0,0]);});
 it('splits merge parents then converges at their shared ancestor',()=>{const r=buildGraph([c('m',['a','b']),c('a',['r']),c('b',['r']),c('r',[])]);expect(r[0].edges.map(e=>e.to)).toEqual([0,1]);expect(r[2].lane).toBe(1);expect(r[3].incoming).toHaveLength(2);expect(r[3].edges).toHaveLength(0);});
});
