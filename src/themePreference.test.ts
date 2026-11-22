import {expect,test,vi} from 'vitest';
import {loadTheme,saveTheme,observeTheme,THEME_KEY} from './themePreference';
test('existing choices survive restart and unsupported saved data is never rewritten on read',()=>{
 let value:string|null=null;const storage={getItem:vi.fn(()=>value),setItem:vi.fn((_key:string,next:string)=>{value=next;})};
 expect(loadTheme(()=>storage)).toEqual({theme:'dark',warning:''});
 for(const theme of ['dark','light','system'] as const){expect(saveTheme(()=>storage,theme)).toBe('');expect(loadTheme(()=>storage)).toEqual({theme,warning:''});}
 value='unexpected';const writes=storage.setItem.mock.calls.length;expect(loadTheme(()=>storage).theme).toBe('dark');expect(loadTheme(()=>storage).warning).not.toBe('');expect(storage.setItem.mock.calls.length).toBe(writes);expect(storage.getItem).toHaveBeenCalledWith(THEME_KEY);
});
test('storage getter, reads and quota failures do not prevent startup or a session theme choice',()=>{
 const denied=()=>{throw new Error('denied');};expect(loadTheme(denied).theme).toBe('dark');expect(saveTheme(denied,'light')).toContain('could not be saved');
 const broken={getItem:()=>{throw new Error('read');},setItem:()=>{throw new Error('quota');}};expect(loadTheme(()=>broken).warning).toContain('could not be read');expect(saveTheme(()=>broken,'system')).toContain('session');
 const recovered={getItem:()=>null,setItem:vi.fn()};expect(saveTheme(()=>recovered,'system')).toBe('');
});
test('system follows live appearance changes and detaches on switch or unmount',()=>{
 const listeners=new Set<()=>void>();const query={matches:false,addEventListener:(_type:string,listener:unknown)=>{listeners.add(listener as ()=>void);},removeEventListener:(_type:string,listener:unknown)=>{listeners.delete(listener as ()=>void);}};
 const apply=vi.fn();const stop=observeTheme('system',()=>query,apply);expect(apply).toHaveBeenLastCalledWith('light');query.matches=true;listeners.forEach(fn=>fn());expect(apply).toHaveBeenLastCalledWith('dark');stop();expect(listeners.size).toBe(0);
 observeTheme('light',()=>{throw new Error('must not query OS');},apply);query.matches=false;listeners.forEach(fn=>fn());expect(apply).toHaveBeenLastCalledWith('light');
 const again=observeTheme('system',()=>query,apply);expect(listeners.size).toBe(1);again();expect(listeners.size).toBe(0);
});
test('unavailable system appearance falls back without blocking application startup',()=>{
 const apply=vi.fn();const stop=observeTheme('system',()=>{throw new Error('unavailable');},apply);expect(apply).toHaveBeenCalledWith('dark');expect(()=>stop()).not.toThrow();
});
