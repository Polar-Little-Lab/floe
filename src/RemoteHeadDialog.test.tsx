import {expect,test} from 'vitest';
import {renderToStaticMarkup} from 'react-dom/server';
import {RemoteHeadDialog} from './RemoteHeadDialog';
const props={name:'origin',branches:['main','next'],current:'next',busy:false,error:'',onClose:()=>{},onSave:()=>{}};
test('remote HEAD editor retains current target and explains local scope',()=>{
 const html=renderToStaticMarkup(<RemoteHeadDialog {...props}/>);
 expect(html).toContain('value="next" selected=""');expect(html).toContain('Current: next');expect(html).toContain('Match server default');expect(html).toContain('Remove local remote HEAD');expect(html).toContain('Changes only the local remote HEAD');
});
test('remote HEAD editor cannot apply missing target and honors busy state',()=>{
 const empty=renderToStaticMarkup(<RemoteHeadDialog {...props} current={null} branches={[]}/>);
 expect(empty).toContain('Fetch a remote branch first');expect(empty).toContain('disabled="">Apply remote HEAD');
 const busy=renderToStaticMarkup(<RemoteHeadDialog {...props} busy error="STALE_STATE"/>);
 expect(busy).toContain('role="alert"');expect(busy).toContain('disabled="">Applying…');expect(busy).toMatch(/<button[^>]*disabled=""[^>]*>Cancel<\/button>/);
});
