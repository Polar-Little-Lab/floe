import {expect,test} from 'vitest';
import {renderToStaticMarkup} from 'react-dom/server';
import {RemoteRemoveDialog} from './RemoteRemoveDialog';
test('remote removal preview distinguishes removed and preserved references and escapes repository text',()=>{
 const html=renderToStaticMarkup(<RemoteRemoveDialog plan={{name:'<origin>',removedRefs:['refs/remotes/origin/main'],preservedRefs:['refs/heads/import/main'],removedConfigKeys:['branch.main.remote']}} busy={false} error="" onClose={()=>{}} onRemove={()=>{}}/>);
 expect(html).toContain('&lt;origin&gt;');expect(html).not.toContain('<origin>');expect(html).toContain('Remove tracking references (1)');expect(html).toContain('Preserve shared or local references');expect(html).toContain('refs/heads/import/main');expect(html).toContain('server repository and local branches stay unchanged');
});
test('remote removal busy and config-only states are explicit',()=>{
 const html=renderToStaticMarkup(<RemoteRemoveDialog plan={{name:'origin',removedRefs:[],preservedRefs:[],removedConfigKeys:['remote.origin.url']}} busy error="STALE_STATE" onClose={()=>{}} onRemove={()=>{}}/>);
 expect(html).toContain('No tracking references will be removed');expect(html).toContain('disabled="">Removing');expect(html).toContain('role="alert"');
});
