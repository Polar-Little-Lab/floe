import {it,expect} from 'vitest';
import {workspaceDraft} from './namedWorkspaces';
it('validates names/repositories and retains known missing local mappings',()=>{
 const draft=workspaceDraft(' Team ','Owner/Repo\nOther/repo',[{owner:'owner',repo:'repo',localPath:'/missing/local'}]);
 expect(draft.name).toBe('Team');expect(draft.repositories[0].localPath).toBe('/missing/local');expect(draft.repositories[1].localPath).toBeNull();
 expect(()=>workspaceDraft('x','o/r\nO/R')).toThrow('Duplicate');expect(()=>workspaceDraft('x','https://github.com/o/r')).toThrow();expect(()=>workspaceDraft('  ','o/r')).toThrow();expect(()=>workspaceDraft('한'.repeat(86),'')).toThrow();
 expect(workspaceDraft('Empty','').repositories).toEqual([]);
});
