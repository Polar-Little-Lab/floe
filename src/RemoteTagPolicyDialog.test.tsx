import {expect,test} from 'vitest';
import {renderToStaticMarkup} from 'react-dom/server';
import {RemoteTagPolicyDialog} from './RemoteTagPolicyDialog';
import {RemoteList} from './RemoteList';
test('tag policy editor displays current supported policy and preserves endpoints',()=>{
 const html=renderToStaticMarkup(<RemoteTagPolicyDialog name="<origin>" initial="all" busy={false} error="" onClose={()=>{}} onSave={()=>{}}/>);
 expect(html).toContain('&lt;origin&gt;');expect(html).toContain('value="all" selected=""');
 expect(html).toContain('Explicit tag fetch rules still apply');expect(html).toContain('Remote URLs and fetch rules are preserved');
 expect(html).not.toContain('Remote fetch URL');expect(html).not.toContain('disabled="">Save tag policy');
});
test('unsupported policy requires explicit choice and busy disables submission',()=>{
 const props={name:'origin',busy:false,error:'STALE_STATE',onClose:()=>{},onSave:()=>{}};
 const unsupported=renderToStaticMarkup(<RemoteTagPolicyDialog {...props} initial="unsupported"/>);
 expect(unsupported).toContain('Unsupported current setting');expect(unsupported).toContain('disabled="">Save tag policy');expect(unsupported).toContain('role="alert"');
 const busy=renderToStaticMarkup(<RemoteTagPolicyDialog {...props} initial="none" busy/>);
 expect(busy).toContain('disabled="">Saving…');expect(busy).toContain('aria-label="Tags to fetch" disabled=""');
});
test('remote tag settings use a busy-guarded context menu',()=>{
 const props={remotes:[{name:'origin',tagPolicy:'auto',fetchUrls:[],pushUrls:[],fetchRefspecs:[],branches:[]}]};
 expect(renderToStaticMarkup(<RemoteList {...props}/>)).not.toContain('Configure fetched tags');
 expect(renderToStaticMarkup(<RemoteList {...props} busy onTagPolicy={()=>{}}/>)).toContain('disabled="" aria-label="Remote actions origin"');
});
