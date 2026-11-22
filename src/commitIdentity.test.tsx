import {expect,test} from 'vitest';
import {renderToStaticMarkup} from 'react-dom/server';
import {repositoryDefaultIdentity,resolveCommitIdentity} from './commitIdentity';
import {useIdentityFields} from './useIdentityFields';
import {IdentitySettingsPanel} from './IdentitySettingsPanel';
const defaults={name:'Default <User>',email:'default@example.invalid'};
test('blank operation uses a complete default pair and partial overrides never mix identities',()=>{
 expect(resolveCommitIdentity('','',defaults)).toEqual(defaults);
 expect(resolveCommitIdentity('Other','',defaults)).toEqual({name:'Other',email:''});
 expect(resolveCommitIdentity('','other@example.invalid',defaults)).toEqual({name:'',email:'other@example.invalid'});
 expect(resolveCommitIdentity('Other','other@example.invalid',defaults)).toEqual({name:'Other',email:'other@example.invalid'});
 expect(resolveCommitIdentity('','',null)).toEqual({name:'',email:''});
});
test('operation fields display escaped saved identity and allow use without a profile',()=>{
 function Fields({saved=true}:{saved?:boolean}){const fields=useIdentityFields(saved?defaults:null);return <><input aria-label="name" value={fields.author} readOnly/><input aria-label="email" value={fields.email} readOnly/></>;}
 const html=renderToStaticMarkup(<Fields/>);expect(html).toContain('Default &lt;User&gt;');expect(html).toContain('default@example.invalid');expect(renderToStaticMarkup(<Fields saved={false}/>)).not.toContain('default@example.invalid');
});
test('identity persistence is unavailable in browser preview and never claims to update Git config',()=>{
 const html=renderToStaticMarkup(<IdentitySettingsPanel onSaved={()=>{}}/>);expect(html).toContain('Available in the desktop app');expect(html).toContain('Git configuration is unchanged');expect(html).toContain('disabled="">Save identity');
});

test('repository identity overrides app defaults; damaged partial identity never silently falls back',()=>{
 const local={name:'Work',email:'work@example.invalid'};
 expect(repositoryDefaultIdentity({identity:local,configured:true,warning:null},defaults)).toEqual(local);
 expect(repositoryDefaultIdentity({identity:null,configured:false,warning:null},defaults)).toEqual(defaults);
 expect(repositoryDefaultIdentity({identity:null,configured:true,warning:'invalid'},defaults)).toBeNull();
 expect(resolveCommitIdentity('Explicit','explicit@example.invalid',local)).toEqual({name:'Explicit',email:'explicit@example.invalid'});
 expect(resolveCommitIdentity('Partial','',local)).toEqual({name:'Partial',email:''});
});
