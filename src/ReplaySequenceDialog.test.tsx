import {expect,test} from 'vitest';
import {renderToStaticMarkup} from 'react-dom/server';
import {ReplaySequenceDialog} from './ReplaySequenceDialog';
import type {Commit,ReplaySequencePlan} from './types';
const initial:Commit={oid:'a'.repeat(40),parents:[],summary:'<source>',author:'Test',time:0,refs:[]};
const props={initial,commits:[initial],plan:null,busy:false,error:'',onPreview:()=>{},onReset:()=>{},onClose:()=>{}};
test('ordered preview starts with one explicit source and requires merge mainline',()=>{
 const html=renderToStaticMarkup(<ReplaySequenceDialog {...props}/>);expect(html).toContain('Sequence commit 1');expect(html).toContain('&lt;source&gt;');expect(html).toContain('미리보기는 저장소를 변경하지 않습니다');expect(html).not.toContain('Apply sequence');
 const merged={...initial,parents:['b','c']};const merge=renderToStaticMarkup(<ReplaySequenceDialog {...props} initial={merged} commits={[merged]}/>);expect(merge).toContain('Choose a parent');expect(merge).toContain('disabled="">Preview sequence');
});
test('conflict stop reports uncalculated steps and escapes intermediate paths',()=>{
 const plan:ReplaySequencePlan={revision:'r',head:'h',requested:3,stoppedAt:0,resultTree:null,blockers:['CONFLICTS'],steps:[{selection:{oid:initial.oid,kind:'cherryPick',mainline:null},beforeTree:'b',resultTree:null,parent:null,originallyEmpty:false,empty:false,conflicts:[{ancestor:null,ours:{id:'f',path:'<conflict>',oid:'c'},theirs:null}],diff:null}]};
 const html=renderToStaticMarkup(<ReplaySequenceDialog {...props} plan={plan} busy error="<failure>"/>);expect(html).toContain('Stopped at step 1. 2 later steps');expect(html).toContain('&lt;conflict&gt;');expect(html).toContain('&lt;failure&gt;');expect(html).toContain('disabled="">Calculating…');
});
