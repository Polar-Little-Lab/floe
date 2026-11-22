import {expect,test} from 'vitest';
import {renderToStaticMarkup} from 'react-dom/server';
import {RepositoryIdentityDialog} from './RepositoryIdentityDialog';
const props={name:'<work>',initial:{identity:{name:'Work User',email:'work@example.invalid'},configured:true,warning:null},busy:false,error:'',onClose:()=>{},onSave:()=>{}};
test('repository identity editor explains local scope and uses existing pair',()=>{
 const html=renderToStaticMarkup(<RepositoryIdentityDialog {...props}/>);expect(html).toContain('&lt;work&gt;');expect(html).toContain('value="Work User"');expect(html).toContain('user.name and user.email');expect(html).not.toContain('disabled="">Save repository identity');expect(html).toContain('Global Git configuration and your app default are unchanged');
});
test('invalid/missing pair cannot save; busy cannot clear or close',()=>{
 const empty=renderToStaticMarkup(<RepositoryIdentityDialog {...props} initial={{identity:null,configured:true,warning:'Repair identity'}}/>);expect(empty).toContain('Repair identity');expect(empty).toContain('disabled="">Save repository identity');
 const busy=renderToStaticMarkup(<RepositoryIdentityDialog {...props} busy error="STALE_STATE"/>);expect(busy).toContain('disabled="">Clear repository identity');expect(busy).toContain('disabled="">Saving…');expect(busy).toContain('role="alert"');
});
