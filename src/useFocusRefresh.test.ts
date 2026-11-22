import {afterEach,beforeEach,expect,test,vi} from 'vitest';
import {useFocusRefresh} from './useFocusRefresh';

const lifecycle=vi.hoisted(()=>({cleanup:undefined as undefined|(()=>void)}));
vi.mock('react',()=>({
 useRef:(current:unknown)=>({current}),
 useEffect:(effect:()=>void|(()=>void))=>{lifecycle.cleanup=effect()||undefined;},
}));
let win:EventTarget;
let doc:EventTarget & {hidden:boolean;hasFocus:()=>boolean;querySelector:()=>object|null};
beforeEach(()=>{
 vi.useFakeTimers();win=new EventTarget();
 doc=Object.assign(new EventTarget(),{hidden:false,hasFocus:()=>true,querySelector:():object|null=>null});
 vi.stubGlobal('window',win);vi.stubGlobal('document',doc);
});
afterEach(()=>{lifecycle.cleanup?.();lifecycle.cleanup=undefined;vi.useRealTimers();vi.unstubAllGlobals();});
const focus=()=>win.dispatchEvent(new Event('focus'));

test('native confirmation focus waits for mutation completion and coalesces repeated events',async()=>{
 let busy=true;const refresh=vi.fn().mockResolvedValue(undefined);
 useFocusRefresh('fixture',true,refresh,()=>busy);
 focus();focus();await vi.advanceTimersByTimeAsync(900);
 expect(refresh).not.toHaveBeenCalled();
 busy=false;await vi.advanceTimersByTimeAsync(300);
 expect(refresh).toHaveBeenCalledTimes(1);
 await vi.advanceTimersByTimeAsync(900);expect(refresh).toHaveBeenCalledTimes(1);
});

test('review dialog blocks refresh until it closes',async()=>{
 doc.querySelector=()=>({});const refresh=vi.fn().mockResolvedValue(undefined);
 useFocusRefresh('fixture',true,refresh,()=>false);focus();
 await vi.advanceTimersByTimeAsync(600);expect(refresh).not.toHaveBeenCalled();
 doc.querySelector=()=>null;await vi.advanceTimersByTimeAsync(300);
 expect(refresh).toHaveBeenCalledTimes(1);
});

test('focus during a pending refresh queues only one additional refresh',async()=>{
 let finish!:()=>void;
 const refresh=vi.fn().mockImplementationOnce(()=>new Promise<void>(resolve=>{finish=resolve;})).mockResolvedValue(undefined);
 useFocusRefresh('fixture',true,refresh,()=>false);focus();await vi.advanceTimersByTimeAsync(300);
 focus();focus();await vi.advanceTimersByTimeAsync(900);expect(refresh).toHaveBeenCalledTimes(1);
 finish();await vi.advanceTimersByTimeAsync(300);expect(refresh).toHaveBeenCalledTimes(2);
});

test('a failed refresh is not automatically retried',async()=>{
 const refresh=vi.fn().mockRejectedValue(new Error('unavailable'));
 useFocusRefresh('fixture',true,refresh,()=>false);focus();await vi.advanceTimersByTimeAsync(3000);
 expect(refresh).toHaveBeenCalledTimes(1);
});

test('hidden document defers work until visible and focused',async()=>{
 const refresh=vi.fn().mockResolvedValue(undefined);
 useFocusRefresh('fixture',true,refresh,()=>false);focus();doc.hidden=true;
 doc.dispatchEvent(new Event('visibilitychange'));await vi.advanceTimersByTimeAsync(600);
 expect(refresh).not.toHaveBeenCalled();doc.hidden=false;doc.hasFocus=()=>false;
 doc.dispatchEvent(new Event('visibilitychange'));await vi.advanceTimersByTimeAsync(300);
 expect(refresh).not.toHaveBeenCalled();doc.hasFocus=()=>true;focus();await vi.advanceTimersByTimeAsync(300);
 expect(refresh).toHaveBeenCalledTimes(1);
});

test('repository teardown removes pending callbacks and listeners',async()=>{
 const refresh=vi.fn().mockResolvedValue(undefined);
 useFocusRefresh('fixture',true,refresh,()=>false);focus();lifecycle.cleanup?.();focus();
 await vi.advanceTimersByTimeAsync(600);expect(refresh).not.toHaveBeenCalled();
});

test('disabled refresh registers no work',async()=>{
 const refresh=vi.fn();useFocusRefresh('demo',false,refresh,()=>false);focus();
 await vi.advanceTimersByTimeAsync(600);expect(refresh).not.toHaveBeenCalled();
});
