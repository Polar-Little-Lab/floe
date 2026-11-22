import {it,expect} from 'vitest';
import {flushBeforeExit} from './shutdownFlow';
it('acknowledges exit only after pending save completes',async()=>{
 const events:string[]=[];let release:()=>void=()=>{};const wait=new Promise<void>(r=>{release=r;});
 const task=flushBeforeExit(async()=>{events.push('saving');await wait;events.push('saved');},async saved=>{events.push(`exit:${saved}`);});
 expect(events).toEqual(['saving']);release();await task;expect(events).toEqual(['saving','saved','exit:true']);
});
it('denies quit on save conflict and permits a later successful retry',async()=>{
 const acknowledgements:boolean[]=[];const finish=async(saved:boolean)=>{acknowledgements.push(saved);};
 await expect(flushBeforeExit(async()=>{throw Error('STALE');},finish)).rejects.toThrow('STALE');
 await flushBeforeExit(async()=>{},finish);expect(acknowledgements).toEqual([false,true]);
});
it('reports a refused backend completion instead of claiming exit',async()=>{
 await expect(flushBeforeExit(async()=>{},async()=>{throw Error('BUSY');})).rejects.toThrow('BUSY');
});
