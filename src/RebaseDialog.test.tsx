import {expect,test} from 'vitest';
import {renderToStaticMarkup} from 'react-dom/server';
import {RebaseDialog} from './RebaseDialog';
import type {RebasePlan} from './types';
const plan:RebasePlan={revision:'r',branch:'topic',upstream:'main',headOid:'a'.repeat(40),upstreamOid:'b'.repeat(40),baseOid:'c'.repeat(40),kind:'replay',selections:[],skippedEquivalent:[],flattenedMerges:[],replay:null,resultTree:'d'.repeat(40),blockers:[]};
const props={plan,busy:false,error:'',defaultIdentity:{name:'Test',email:'t@invalid'},onClose:()=>{},onApply:()=>{}};
test('rebase preview explains rewritten history and committer identity',()=>{const html=renderToStaticMarkup(<RebaseDialog {...props}/>);expect(html).toContain('Rewrites this local branch');expect(html).toContain('Original authors and messages');expect(html).toContain('value="Test"');expect(html).toContain('class="primary">Rebase branch');});
test('rebase refuses conflicts dirty state noop missing identity and busy submit',()=>{for(const blockers of [['CONFLICTS'],['DIRTY_WORKTREE']]){expect(renderToStaticMarkup(<RebaseDialog {...props} plan={{...plan,blockers}}/>)).toContain('disabled="">Rebase branch');}expect(renderToStaticMarkup(<RebaseDialog {...props} plan={{...plan,kind:'upToDate'}}/>)).toContain('Already up to date.');expect(renderToStaticMarkup(<RebaseDialog {...props} defaultIdentity={null}/>)).toContain('disabled="">Rebase branch');expect(renderToStaticMarkup(<RebaseDialog {...props} busy/>)).toContain('disabled="">Rebasing…');});
