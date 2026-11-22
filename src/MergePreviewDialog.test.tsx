import { expect,test } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { MergePreviewDialog } from './MergePreviewDialog';
import type { MergePlan } from './types';
test('merge preview distinguishes absent conflict stages and does not present execution as available',()=>{
 const plan:MergePlan={revision:'r',currentBranch:'main',targetBranch:'<topic>',headOid:'1111111',targetOid:'2222222',baseOid:'3333333',kind:'conflicts',blockers:['<blocked>'],files:[],conflicts:[{ancestor:{id:'base',path:'a.txt',oid:'abc1234'},ours:{id:'ours',path:'<script>.txt',oid:'def5678'},theirs:null}],diff:null};
 const html=renderToStaticMarkup(<MergePreviewDialog plan={plan} onClose={()=>{}}/>);
 expect(html).toContain('Conflicts require resolution');expect(html).toContain('Incoming:');expect(html).toContain('Absent');expect(html).toContain('Conflict resolutions are saved as a draft until Continue applies them.');expect(html).not.toContain('<script>');expect(html).not.toContain('<topic>');expect(html).not.toContain('<blocked>');
 expect(html.match(/<button/g)).toHaveLength(1);
});

test('only unblocked fast-forward plans enable applying',()=>{
 const plan:MergePlan={revision:'r',currentBranch:'main',targetBranch:'topic',headOid:'1111111',targetOid:'2222222',baseOid:'1111111',kind:'fastForward',blockers:[],files:[],conflicts:[],diff:null};
 const render=(p:MergePlan,busy=false)=>renderToStaticMarkup(<MergePreviewDialog plan={p} onClose={()=>{}} onApply={()=>{}} busy={busy}/>);
 expect(render(plan)).toContain('<button>Apply fast-forward merge</button>');
 expect(render({...plan,blockers:['dirty']})).toContain('<button disabled="">Apply fast-forward merge</button>');
 expect(render(plan,true)).toContain('<button disabled="">Apply fast-forward merge</button>');
 expect(render({...plan,kind:'mergeCommit'})).not.toContain('Apply fast-forward merge');
});

test('merge commit requires explicit identity and does not offer conflict execution',()=>{
 const plan:MergePlan={revision:'r',currentBranch:'main',targetBranch:'topic',headOid:'1111111',targetOid:'2222222',baseOid:'3333333',kind:'mergeCommit',blockers:[],files:[],conflicts:[],diff:null};
 const render=(p:MergePlan)=>renderToStaticMarkup(<MergePreviewDialog plan={p} onClose={()=>{}} onCommit={()=>{}}/>);
 expect(render(plan)).toContain('aria-label="Merge author"');
 expect(render(plan)).toContain('<button type="submit" disabled="">Create merge commit</button>');
 expect(render({...plan,kind:'conflicts'})).not.toContain('Create merge commit');
});

test('conflict versions have scoped read controls and escape displayed source content',()=>{
 const plan:MergePlan={revision:'r',currentBranch:'main',targetBranch:'topic',headOid:'1111111',targetOid:'2222222',baseOid:'3333333',kind:'conflicts',blockers:[],files:[],conflicts:[{ancestor:{id:'a',path:'file',oid:'aaaaaaa'},ours:{id:'a',path:'file',oid:'bbbbbbb'},theirs:null}],diff:null};
 const html=renderToStaticMarkup(<MergePreviewDialog plan={plan} onClose={()=>{}} onRead={()=>{}} busy content={{path:'file',source:'Base',state:'text',size:8,text:'<script>',truncated:false,endsWithNewline:false}}/>);
 expect(html).toContain('<button disabled="">Read Base</button>');expect(html).toContain('Read Current');expect(html).not.toContain('Read Incoming');
 expect(html).toContain('&lt;script&gt;');expect(html).not.toContain('<script>');expect(html).toContain('No newline at end of file');
});

test('pending merge exposes saved resolutions, continue and abort controls',()=>{
 const plan:MergePlan={revision:'r',currentBranch:'main',targetBranch:'topic',headOid:'1111111',targetOid:'2222222',baseOid:'3333333',kind:'conflicts',blockers:[],files:[],conflicts:[{ancestor:null,ours:{id:'a',path:'file',oid:'bbbbbbb'},theirs:null}],diff:null};
 const html=renderToStaticMarkup(<MergePreviewDialog plan={plan} onClose={()=>{}} draft={{name:'topic',branch:'main',head:'1111111',target:'2222222',resolutions:{a:{kind:'text',text:'<safe>'}}}} onContinue={()=>{}} onResolve={()=>{}} onAbort={()=>{}}/>);
 expect(html).toContain('Saved resolution: text');expect(html).toContain('&lt;safe&gt;');expect(html).toContain('Continue merge');expect(html).toContain('Abort merge');expect(html).toContain('Use current');expect(html).toContain('Save edited resolution');
});

test('remote merge labels hide internal selector and blocked conflicts cannot start',()=>{
 const plan:MergePlan={revision:'r',currentBranch:'main',targetBranch:'remote:refs/remotes/origin/main',headOid:'1111111',targetOid:'2222222',baseOid:'3333333',kind:'conflicts',blockers:['blocked'],files:[],conflicts:[],diff:null};
 const html=renderToStaticMarkup(<MergePreviewDialog plan={plan} onClose={()=>{}} onStart={()=>{}}/>);
 expect(html).toContain('origin/main');expect(html).not.toContain('remote:refs/remotes/');expect(html).toContain('<button disabled="">Start conflict resolution</button>');
});
