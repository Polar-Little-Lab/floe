import {expect,test} from 'vitest';
import {renderToStaticMarkup} from 'react-dom/server';
import {SequenceProgressPanel} from './SequenceProgressPanel';
const progress={id:'sequence',branch:'main',originalHead:'a',expectedHead:'b',total:2,completed:1,pending:false};
test('unprepared step requires identity and message and empty is opt-in',()=>{
 const html=renderToStaticMarkup(<SequenceProgressPanel progress={progress} disabled={false} onAction={()=>{}}/>);
 expect(html).toContain('Sequence committer name');expect(html).toContain('disabled="">Apply next step');
 expect(html).toContain('>Skip current step');expect(html).not.toContain('checked=""');expect(html).toContain('Stop &amp; keep commits');expect(html).toContain('원래 HEAD로 되돌리지 않습니다');
});
test('prepared step resumes saved intent and recovery disables both actions',()=>{
 const html=renderToStaticMarkup(<SequenceProgressPanel progress={{...progress,pending:true}} disabled={false} onAction={()=>{}}/>);
 expect(html).toContain('>Resume prepared step');expect(html).not.toContain('Sequence committer name');
 const blocked=renderToStaticMarkup(<SequenceProgressPanel progress={{...progress,pending:true}} disabled onAction={()=>{}}/>);
 expect(blocked).toContain('disabled="">Skip current step');expect(blocked).toContain('disabled="">Resume prepared step');expect(blocked).toContain('disabled="">Stop &amp; keep commits');
});
test('completed sequence offers finish and damaged state can stop without executing',()=>{
 const done=renderToStaticMarkup(<SequenceProgressPanel progress={{...progress,completed:2}} disabled={false} onAction={()=>{}}/>);
 expect(done).not.toContain('Skip current step');expect(done).toContain('>Finish sequence');expect(done).not.toContain('Apply next step');
 const broken=renderToStaticMarkup(<SequenceProgressPanel error="<broken>" disabled={false} onAction={()=>{}}/>);
 expect(broken).toContain('&lt;broken&gt;');expect(broken).toContain('>Stop &amp; keep commits');expect(broken).not.toContain('Apply next step');
});

test('current sequence conflict exposes saved editor and version reads without rendering markup',()=>{
 const current={oid:'source',kind:'cherryPick' as const,mainline:null};
 const plan={revision:'r',head:'h',target:'source',parent:'p',kind:'cherryPick' as const,resultTree:null,conflicts:[{ancestor:null,ours:{id:'f',path:'<conflict>',oid:'ours'},theirs:{id:'f',path:'<conflict>',oid:'theirs'}}],blockers:['CONFLICTS'],empty:false,diff:{text:'',truncated:false}};
 const html=renderToStaticMarkup(<SequenceProgressPanel progress={{...progress,current,resolutions:{f:{kind:'text',text:'<saved>'}}}} plan={plan} disabled={false} onAction={()=>{}} onInspect={()=>{}} onRead={()=>{}}/>);
 expect(html).toContain('Inspect current step');expect(html).toContain('Sequence conflicts');expect(html).toContain('Saved resolution: text');expect(html).toContain('&lt;saved&gt;');expect(html).toContain('&lt;conflict&gt;');expect(html).toContain('Read ours');expect(html).toContain('Read theirs');expect(html).toContain('Save edited resolution');
 const pending=renderToStaticMarkup(<SequenceProgressPanel progress={{...progress,current,pending:true}} plan={plan} disabled={false} onAction={()=>{}} onInspect={()=>{}}/>);
 expect(pending).not.toContain('Save edited resolution');expect(pending).not.toContain('Inspect current step');
});

test('abort in progress exposes only guarded resume and preserves the rewind warning',()=>{
 const html=renderToStaticMarkup(<SequenceProgressPanel progress={{...progress,aborting:true}} disabled={false} onAction={()=>{}} onInspect={()=>{}}/>);
 expect(html).toContain('Resume sequence abort');expect(html).not.toContain('Apply next step');expect(html).not.toContain('Skip current step');expect(html).not.toContain('Inspect current step');expect(html).not.toContain('>Stop &amp; keep commits</button>');
 const blocked=renderToStaticMarkup(<SequenceProgressPanel progress={{...progress,aborting:true}} disabled onAction={()=>{}}/>);
 expect(blocked).toContain('disabled="">Resume sequence abort');
});
