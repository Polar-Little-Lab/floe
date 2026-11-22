import {test,expect} from 'vitest';
import {renderToStaticMarkup} from 'react-dom/server';
import {ReplayPreviewDialog} from './ReplayPreviewDialog';
import type {Commit,ReplayPlan} from './types';
const commit:Commit={oid:'a'.repeat(40),parents:[],summary:'<unsafe>',author:'Test',time:0,refs:[]};
const props={commit,plan:null,error:'',busy:false,onPreview:()=>{},onReset:()=>{},onClose:()=>{}};
test('merge preview requires explicit mainline and exposes read-only scope',()=>{
 const html=renderToStaticMarkup(<ReplayPreviewDialog {...props} commit={{...commit,parents:['b'.repeat(40),'c'.repeat(40)]}}/>);
 expect(html).toContain('Mainline parent');expect(html).toContain('Choose a parent');expect(html).toMatch(/type="submit" disabled=""/);expect(html).toContain('&lt;unsafe&gt;');expect(html).toContain('충돌 없는 결과');
 const root=renderToStaticMarkup(<ReplayPreviewDialog {...props}/>);expect(root).not.toContain('Mainline parent');expect(root).not.toContain('disabled=""');
});
test('busy locks input and conflicts render escaped paths and absent stages',()=>{
 const plan:ReplayPlan={revision:'r',head:'a'.repeat(40),target:commit.oid,parent:null,kind:'revert',resultTree:null,empty:false,blockers:['CONFLICTS'],conflicts:[{ancestor:null,ours:{id:'f',path:'<file>',oid:'b'.repeat(40)},theirs:null}],diff:{text:'',truncated:false,hunks:[]}};
 const html=renderToStaticMarkup(<ReplayPreviewDialog {...props} busy plan={plan} error="<error>"/>);
 expect(html.match(/disabled=""/g)).toHaveLength(3);expect(html).toContain('Calculating…');expect(html).toContain('Conflicts');expect(html).toContain('&lt;file&gt;');expect(html).toContain('Absent');expect(html).toContain('&lt;error&gt;');expect(html).toContain('role="alert"');
});

test('apply form is available only for clean nonempty matching result',()=>{
 const plan:ReplayPlan={revision:'r',head:'a',target:commit.oid,parent:null,kind:'cherryPick',resultTree:'tree',conflicts:[],blockers:[],empty:false,diff:{text:'',truncated:false,hunks:[]}};
 const render=(p:ReplayPlan)=>renderToStaticMarkup(<ReplayPreviewDialog {...props} plan={p} onApply={()=>{}}/>);
 expect(render(plan)).toContain('aria-label="Apply replay"');expect(render(plan)).toMatch(/type="submit" disabled="">Apply cherry-pick/);
 expect(render({...plan,empty:true})).not.toContain('aria-label="Apply replay"');expect(render({...plan,blockers:['DIRTY_WORKTREE']})).not.toContain('aria-label="Apply replay"');expect(render({...plan,target:'wrong'})).not.toContain('aria-label="Apply replay"');
});
test('conflict draft gates Continue until every resolution is saved and restores operation',()=>{
 const conflict={ancestor:{id:'f',path:'file',oid:'a'},ours:{id:'f',path:'file',oid:'b'},theirs:{id:'f',path:'file',oid:'c'}};
 const plan:ReplayPlan={revision:'r',head:'h',target:commit.oid,parent:'p',kind:'revert',resultTree:null,conflicts:[conflict],blockers:['CONFLICTS'],empty:false,diff:{text:'',truncated:false,hunks:[]}};
 const draft={version:1,branch:'main',head:'h',source:commit.oid,kind:'revert' as const,mainline:null,resolutions:{}};
 const render=(resolutions:typeof draft.resolutions)=>renderToStaticMarkup(<ReplayPreviewDialog {...props} plan={plan} draft={{...draft,resolutions}} onResolve={()=>{}} onContinue={()=>{}} onAbort={()=>{}}/>);
 const unresolved=render({});expect(unresolved).toContain('Use current');expect(unresolved).toContain('Use incoming');expect(unresolved).toContain('Abort replay');expect(unresolved).not.toContain('Continue replay');expect(unresolved).toContain('value="revert" selected=""');
 const resolved=render({f:{kind:'text',text:'<resolved>'}});expect(resolved).toContain('Continue replay');expect(resolved).toContain('&lt;resolved&gt;');expect(resolved).toContain('Saved resolution: text');
});

test('unresolved index stages are not rendered as a file deletion diff',()=>{
 const plan:ReplayPlan={revision:'r',head:'h',target:commit.oid,parent:'p',kind:'cherryPick',resultTree:null,conflicts:[{ancestor:null,ours:{id:'f',path:'file',oid:'a'},theirs:{id:'f',path:'file',oid:'b'}}],blockers:['CONFLICTS'],empty:false,diff:{text:'deleted file mode 100644',truncated:false,hunks:[]}};
 const html=renderToStaticMarkup(<ReplayPreviewDialog {...props} plan={plan}/>);expect(html).not.toContain('deleted file mode');expect(html).toContain('저장한 내용은 Continue에서 적용됩니다.');
});

test('conflict reading exposes only present versions, locks while busy, and escapes content',()=>{
 const plan:ReplayPlan={revision:'r',head:'h',target:commit.oid,parent:null,kind:'cherryPick',resultTree:null,conflicts:[{ancestor:null,ours:{id:'f',path:'<file>',oid:'a'},theirs:{id:'f',path:'<file>',oid:'b'}}],blockers:['CONFLICTS'],empty:false,diff:{text:'',truncated:false,hunks:[]}};
 const html=renderToStaticMarkup(<ReplayPreviewDialog {...props} plan={plan} busy onRead={()=>{}} content={{path:'<file>',source:'Replay Current',state:'text',size:8,text:'<script>',truncated:false,endsWithNewline:false}}/>);
 expect(html).not.toContain('Read Base');expect(html).toContain('disabled="">Read Current');expect(html).toContain('disabled="">Read Incoming');expect(html).toContain('aria-label="Replay conflict version content"');expect(html).toContain('&lt;script&gt;');expect(html).not.toContain('<script>');
});

test('verified empty resolution offers Skip without commit identity or Continue',()=>{
 const plan:ReplayPlan={revision:'r',head:'h',target:commit.oid,parent:'p',kind:'cherryPick',resultTree:null,conflicts:[{ancestor:null,ours:{id:'f',path:'file',oid:'a'},theirs:{id:'f',path:'file',oid:'b'}}],blockers:['CONFLICTS'],empty:false,resolvedEmpty:true,diff:{text:'',truncated:false,hunks:[]}};
 const draft={version:1,branch:'main',head:'h',source:commit.oid,kind:'cherryPick' as const,mainline:null,resolutions:{f:{kind:'ours' as const}}};
 const html=renderToStaticMarkup(<ReplayPreviewDialog {...props} plan={plan} draft={draft} onSkip={()=>{}} onContinue={()=>{}}/>);
 expect(html).toContain('Skip empty replay');expect(html).toContain('Saved conflict resolutions');expect(html).toContain('Keep empty replay commit');expect(html).not.toContain('checked=""');expect(html).not.toContain('충돌 해결이 필요합니다.');expect(html).not.toContain('저장한 내용은 Continue에서 적용됩니다.');expect(html).not.toContain('Continue replay');expect(html).not.toContain('Replay committer');
 const nonempty=renderToStaticMarkup(<ReplayPreviewDialog {...props} plan={{...plan,resolvedEmpty:false}} draft={draft} onSkip={()=>{}} onContinue={()=>{}}/>);expect(nonempty).not.toContain('Skip empty replay');expect(nonempty).toContain('Continue replay');
});


test('empty preview distinguishes originally empty and already-applied changes with opt-in off',()=>{
 const plan:ReplayPlan={revision:'r',head:'h',target:commit.oid,parent:'p',kind:'cherryPick',resultTree:'tree',conflicts:[],blockers:[],empty:true,originallyEmpty:true,diff:{text:'',truncated:false,hunks:[]}};
 const render=(p:ReplayPlan)=>renderToStaticMarkup(<ReplayPreviewDialog {...props} plan={p} onApply={()=>{}}/>);
 const original=render(plan);expect(original).toContain('원래부터 파일 변경이 없습니다');expect(original).toContain('Keep empty replay commit');expect(original).not.toContain('checked=""');expect(original).not.toContain('aria-label="Apply replay"');
 expect(render({...plan,originallyEmpty:false})).toContain('작업의 결과가 현재 HEAD와 같습니다');expect(render({...plan,blockers:['DIRTY_WORKTREE']})).not.toContain('Keep empty replay commit');
});
