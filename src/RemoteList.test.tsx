import {expect,test} from 'vitest';
import {renderToStaticMarkup} from 'react-dom/server';
import {RemoteList} from './RemoteList';
const remotes=[{name:'<origin>',fetchUrls:['https://example.invalid/repo'],pushUrls:[],fetchRefspecs:[],branches:[{name:'main',oid:'a'.repeat(40),symbolicTarget:null},{name:'old',oid:'b'.repeat(40),symbolicTarget:null},{name:'HEAD',oid:null,symbolicTarget:'refs/remotes/origin/main'}]}];
test('remote tree escapes names and keeps endpoints and operation buttons out of sidebar',()=>{
 const html=renderToStaticMarkup(<RemoteList remotes={remotes} onFetch={()=>{}} onPull={()=>{}} onForgetCredential={()=>{}}/>);
 expect(html).toContain('&lt;origin&gt;');expect(html).not.toContain('<origin>');expect(html).toContain('refs/remotes/origin/main');
 expect(html).toContain('Remote actions &lt;origin&gt;');expect(html).not.toContain('https://example.invalid/repo');expect(html).not.toContain('Forget saved login');expect(html).not.toContain('>Fetch</button>');expect(html).not.toContain('>Pull</button>');
});
test('empty and filtered remote states are explicit',()=>{
 expect(renderToStaticMarkup(<RemoteList remotes={[]}/>)).toContain('No configured remotes.');
 const missing=renderToStaticMarkup(<RemoteList remotes={remotes} filter="missing"/>);expect(missing).toContain('No matching remote references.');expect(missing).not.toContain('remote-tree-group');
 const filtered=renderToStaticMarkup(<RemoteList remotes={remotes} filter="old"/>);expect(filtered).toContain('View remote branch &lt;origin&gt;/old');expect(filtered).not.toContain('View remote branch &lt;origin&gt;/main');
});
test('graph navigation retains unloaded and busy restrictions independently of context menu',()=>{
 const props={remotes,visibleOids:['a'.repeat(40)],onSelect:()=>{}};
 const html=renderToStaticMarkup(<RemoteList {...props}/>);expect(html).toContain('disabled="" title="Commit is outside the loaded graph"');expect(html).toContain('title="View remote commit"');
 const busy=renderToStaticMarkup(<RemoteList {...props} busy/>);expect(busy).toContain('disabled="" title="View remote commit"');expect(busy).toContain('disabled="" aria-label="Remote actions &lt;origin&gt;"');
 const all=renderToStaticMarkup(<RemoteList {...props} allowUnloaded/>);expect(all).not.toContain('Commit is outside the loaded graph');
});
