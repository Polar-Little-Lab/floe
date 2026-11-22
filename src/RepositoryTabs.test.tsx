import {expect,test} from 'vitest';
import {renderToStaticMarkup} from 'react-dom/server';
import {RepositoryTabs} from './RepositoryTabs';
import {hasCommitDraft,restoreDraft} from './tabDraft';
test('same-named repositories have distinct handles and all controls lock while busy',()=>{
 const html=renderToStaticMarkup(<RepositoryTabs tabs={[{repoId:'a',name:'project'},{repoId:'b',name:'project'}]} active="b" busy onSelect={()=>{}} onClose={()=>{}}/>);
 expect(html.match(/role="tab"/g)).toHaveLength(2);expect(html.match(/aria-selected="true"/g)).toHaveLength(1);expect(html.match(/disabled=""/g)).toHaveLength(4);
});
test('draft text survives tab switch but amend clears if HEAD changed externally',()=>{
 const draft={message:'saved',description:'body',author:'Test',email:'test@example.invalid',amend:true,headOid:'old'};
 expect(restoreDraft(draft,'old')).toEqual(draft);
 expect(restoreDraft(draft,'new')).toEqual({...draft,amend:false});
 expect(restoreDraft(draft,null).amend).toBe(false);
 expect(hasCommitDraft(draft)).toBe(true);expect(hasCommitDraft(undefined)).toBe(false);
 expect(restoreDraft(undefined,'new').message).toBe('');
});
