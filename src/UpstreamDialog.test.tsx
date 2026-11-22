import {expect,test} from 'vitest';
import {renderToStaticMarkup} from 'react-dom/server';
import {UpstreamDialog} from './UpstreamDialog';
const base={branch:{name:'topic',oid:'1'.repeat(40),current:true,upstream:{remotes:['origin'],merges:['refs/heads/release/v2']}},remotes:[{name:'origin',fetchUrls:[],pushUrls:[],fetchRefspecs:[],branches:[]}],busy:false,error:'',onClose:()=>{},onSave:()=>{}};
test('upstream editor shows current config and supports unfetched branch',()=>{
 const html=renderToStaticMarkup(<UpstreamDialog {...base}/>);
 expect(html).toContain('value="release/v2"');expect(html).toContain('value="origin" selected=""');expect(html).not.toContain('disabled="">Save upstream');expect(html).toContain('no network request starts');
});
test('ambiguous or removed remote requires choice; busy blocks edits and clearing',()=>{
 const ambiguous=renderToStaticMarkup(<UpstreamDialog {...base} branch={{...base.branch,upstream:{remotes:['origin'],merges:['refs/heads/one','refs/heads/two']}}}/>);
 expect(ambiguous).toContain('refs/heads/one, refs/heads/two');expect(ambiguous).toContain('disabled="">Save upstream');
 expect(renderToStaticMarkup(<UpstreamDialog {...base} remotes={[]}/>)).toContain('(not configured)');
 const busy=renderToStaticMarkup(<UpstreamDialog {...base} busy error="STALE_STATE"/>);
 expect(busy).toContain('disabled="">Clear upstream');expect(busy).toContain('disabled="">Saving…');expect(busy).toContain('role="alert"');
});
