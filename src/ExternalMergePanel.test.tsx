import { expect,test } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { ExternalMergePanel } from './ExternalMergePanel';
test('external Git merge distinguishes missing stages and gates mutation controls',()=>{
 const html=renderToStaticMarkup(<ExternalMergePanel merge={{heads:['123456789'],conflicts:[{ancestor:null,ours:{id:'a',path:'<script>.txt',oid:'abcdef'},theirs:{id:'a',path:'<script>.txt',oid:'123456'}}]}} disabled content={null} onRead={()=>{}} onStage={()=>{}}/>);
 expect(html).toContain('&lt;script&gt;.txt');expect(html).not.toContain('<script>');expect(html).toContain('ancestor: Absent');expect(html).toContain('<button disabled="">Stage resolved working file</button>');expect(html).toContain('Continue commits the staged result. Abort discards merge resolutions and restores affected paths to HEAD; unrelated working edits are preserved.');
});

test('external continue requires identity and exposes explicit staged-commit semantics',()=>{
 const html=renderToStaticMarkup(<ExternalMergePanel merge={{heads:['123456789'],conflicts:[]}} disabled={false} content={null} onRead={()=>{}} onStage={()=>{}} onContinue={()=>{}}/>);
 expect(html).toContain('aria-label="External merge author"');expect(html).toContain('aria-label="External merge message"');expect(html).toContain('<button type="submit" disabled="">Continue external merge</button>');expect(html).toContain('Continue commits the staged result.');
});

test('external abort exposes its discard behavior and respects busy/read-only state',()=>{
 const html=renderToStaticMarkup(<ExternalMergePanel merge={{heads:['123456789'],conflicts:[]}} disabled content={null} onRead={()=>{}} onStage={()=>{}} onAbort={()=>{}}/>);
 expect(html).toContain('<button disabled="">Abort external merge</button>');expect(html).toContain('Abort discards merge resolutions');
});
