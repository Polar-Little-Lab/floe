import {expect,test} from 'vitest';
import {renderToStaticMarkup} from 'react-dom/server';
import {RemoteRenameDialog} from './RemoteRenameDialog';
import {RemoteList} from './RemoteList';
test('remote rename explains custom destination handling and escapes names',()=>{
 const html=renderToStaticMarkup(<RemoteRenameDialog name="<origin>" busy={false} error="" onClose={()=>{}} onRename={()=>{}}/>);
 expect(html).toContain('&lt;origin&gt;');expect(html).not.toContain('<origin>');expect(html).toContain('Custom fetch destinations retain their existing paths');expect(html).toContain('disabled="">Rename remote');
});
test('rename cannot be submitted or dismissed during mutation',()=>{
 const html=renderToStaticMarkup(<RemoteRenameDialog name="origin" busy error="STALE_STATE" onClose={()=>{}} onRename={()=>{}}/>);
 expect(html).toContain('disabled="">Renaming');expect(html).toContain('role="alert"');expect(html).toMatch(/<button(?=[^>]*disabled="")(?=[^>]*aria-label="Close remote rename")[^>]*>/);
});
test('remote rename entry uses a busy-guarded context menu',()=>{
 const remotes=[{name:'origin',fetchUrls:[],pushUrls:[],fetchRefspecs:[],branches:[]}];
 expect(renderToStaticMarkup(<RemoteList remotes={remotes}/>)).not.toContain('Rename remote origin');
 expect(renderToStaticMarkup(<RemoteList remotes={remotes} busy onRename={()=>{}}/>)).toContain('disabled="" aria-label="Remote actions origin"');
});
