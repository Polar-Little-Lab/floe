import {expect,test,vi} from 'vitest';
import {applyStageSelection,type StageSelection} from './stageSelection';
import {demo} from './demo';

function fixture(staged=false){
 const selection:StageSelection={snapshot:{...demo,repoId:'selected-fixture',revision:'reviewed'},files:[{id:'opaque-a',path:'a.txt',staged,unstaged:!staged,status:'modified'}],staged};
 const next={...selection.snapshot,revision:'current'};
 const backend={mutate:vi.fn().mockResolvedValue(next),refresh:vi.fn().mockResolvedValue(next)};
 return {selection,next,backend,onRefresh:vi.fn(),onInvalidated:vi.fn()};
}
test.each([false,true])('submits exactly reviewed IDs and revision for staged=%s without background refresh',async staged=>{
 const f=fixture(staged);
 expect(await applyStageSelection(f.selection,f.backend,f.onRefresh,f.onInvalidated)).toBe(f.next);
 expect(f.backend.mutate.mock.calls).toEqual([[f.selection.snapshot,{kind:staged?'unstageFiles':'stageFiles',file_ids:['opaque-a']}]]);
 expect(f.backend.refresh).not.toHaveBeenCalled();expect(f.onInvalidated).not.toHaveBeenCalled();
});
test('stale refusal refreshes the list, invalidates review and never retries against new file IDs',async()=>{
 const f=fixture();f.next.changes=[{...f.selection.files[0],id:'replacement-id'}];
 f.backend.mutate.mockRejectedValue('STALE_STATE: changed');
 await expect(applyStageSelection(f.selection,f.backend,f.onRefresh,f.onInvalidated)).rejects.toBe('STALE_STATE: changed');
 expect(f.backend.mutate).toHaveBeenCalledTimes(1);expect(f.backend.refresh).toHaveBeenCalledWith('selected-fixture');
 expect(f.onRefresh).toHaveBeenCalledWith(f.next);expect(f.onInvalidated).toHaveBeenCalled();
 expect(f.selection.snapshot.revision).toBe('reviewed');expect(f.selection.files[0].id).toBe('opaque-a');
});
test('cancellation leaves the review intact without refresh or invalidation',async()=>{
 const f=fixture();f.backend.mutate.mockRejectedValue('CANCELLED: cancelled');
 await expect(applyStageSelection(f.selection,f.backend,f.onRefresh,f.onInvalidated)).rejects.toBe('CANCELLED: cancelled');
 expect(f.backend.refresh).not.toHaveBeenCalled();expect(f.onInvalidated).not.toHaveBeenCalled();
});
test('unchanged lock refusal can be explicitly retried and keeps the original failure',async()=>{
 const f=fixture();f.backend.refresh.mockResolvedValue(f.selection.snapshot);f.backend.mutate.mockRejectedValue('EXTERNAL_LOCK: busy');
 await expect(applyStageSelection(f.selection,f.backend,f.onRefresh,f.onInvalidated)).rejects.toBe('EXTERNAL_LOCK: busy');
 expect(f.onInvalidated).not.toHaveBeenCalled();expect(f.backend.mutate).toHaveBeenCalledTimes(1);
});
test('refresh failure preserves the mutation error and stale invalidation',async()=>{
 const f=fixture();f.backend.mutate.mockRejectedValue('STALE_STATE: changed');f.backend.refresh.mockRejectedValue('READ_FAILED');
 await expect(applyStageSelection(f.selection,f.backend,f.onRefresh,f.onInvalidated)).rejects.toBe('STALE_STATE: changed');
 expect(f.onInvalidated).toHaveBeenCalled();expect(f.onRefresh).not.toHaveBeenCalled();
});
test('review invalidation blocks further mutation requests',async()=>{
 const f=fixture();f.selection.invalidated=true;
 await expect(applyStageSelection(f.selection,f.backend,f.onRefresh,f.onInvalidated)).rejects.toMatch(/^STALE_STATE:/);
 expect(f.backend.mutate).not.toHaveBeenCalled();expect(f.backend.refresh).not.toHaveBeenCalled();
});
test('unexpected refreshed repository is never applied',async()=>{
 const f=fixture();f.backend.mutate.mockRejectedValue('EXTERNAL_LOCK: busy');f.backend.refresh.mockResolvedValue({...f.next,repoId:'different-repository'});
 await expect(applyStageSelection(f.selection,f.backend,f.onRefresh,f.onInvalidated)).rejects.toBe('EXTERNAL_LOCK: busy');
 expect(f.onRefresh).not.toHaveBeenCalled();expect(f.onInvalidated).toHaveBeenCalled();
});
