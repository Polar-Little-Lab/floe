import {afterEach, beforeEach, expect, test, vi} from 'vitest';
import {renderToStaticMarkup} from 'react-dom/server';
import App from './App';
import {api} from './api';
import {demo} from './demo';

const feedback=vi.hoisted(()=>({error:'EXTERNAL_LOCK: index locked',notice:'Previous operation succeeded',setError:vi.fn(),setNotice:vi.fn(),events:[],clear:vi.fn()}));
const focus=vi.hoisted(()=>({refresh:undefined as undefined|(()=>Promise<void>)}));
vi.mock('./ActivityDialog',async importOriginal=>({...await importOriginal<typeof import('./ActivityDialog')>(),useActivityFeedback:()=>feedback}));
vi.mock('./useFocusRefresh',()=>({useFocusRefresh:(_id:string,_enabled:boolean,refresh:()=>Promise<void>)=>{focus.refresh=refresh;}}));
const originalId=demo.repoId;
beforeEach(()=>{vi.stubGlobal('localStorage',{getItem:()=>null});demo.repoId='feedback-fixture';vi.clearAllMocks();focus.refresh=undefined;});
afterEach(()=>{demo.repoId=originalId;vi.restoreAllMocks();vi.unstubAllGlobals();});

test('focus refresh after native confirmation preserves the mutation error',async()=>{
 const refresh=vi.spyOn(api,'refresh').mockResolvedValue({...demo});
 renderToStaticMarkup(<App/>);
 await focus.refresh!();
 expect(refresh).toHaveBeenCalledWith('feedback-fixture');
 expect(feedback.setError).not.toHaveBeenCalled();
});

test('failed focus refresh reports its error without first clearing feedback',async()=>{
 vi.spyOn(api,'refresh').mockRejectedValue('READ_FAILED: repository unavailable');
 renderToStaticMarkup(<App/>);
 await focus.refresh!();
 expect(feedback.setError.mock.calls).toEqual([['READ_FAILED: repository unavailable']]);
});

test('failed operation takes precedence over an old success in the existing status bar',()=>{
 const html=renderToStaticMarkup(<App/>);
 const footer=html.match(/<footer\b[\s\S]*?<\/footer>/)?.[0];
 expect(footer).toContain('EXTERNAL_LOCK: index locked');
 expect(footer).not.toContain('Previous operation succeeded');
 expect(html).toContain('role="alert"');
});
