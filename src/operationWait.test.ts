import { afterEach,expect,test,vi } from 'vitest';
import { watchSlowOperation } from './operationWait';
afterEach(()=>vi.useRealTimers());
test('slow operation hints once after ten seconds without treating elapsed time as completion',()=>{
 vi.useFakeTimers();const notify=vi.fn();const cleanup=watchSlowOperation(notify);
 vi.advanceTimersByTime(9999);expect(notify).not.toHaveBeenCalled();
 vi.advanceTimersByTime(1);expect(notify).toHaveBeenCalledTimes(1);
 vi.advanceTimersByTime(60_000);expect(notify).toHaveBeenCalledTimes(1);cleanup();
});
test('completed or superseded operations cannot produce a late hint',()=>{
 vi.useFakeTimers();const old=vi.fn();const cleanup=watchSlowOperation(old);vi.advanceTimersByTime(5000);cleanup();
 const next=vi.fn();watchSlowOperation(next);vi.advanceTimersByTime(9999);
 expect(old).not.toHaveBeenCalled();expect(next).not.toHaveBeenCalled();
 vi.advanceTimersByTime(1);expect(next).toHaveBeenCalledTimes(1);
});
