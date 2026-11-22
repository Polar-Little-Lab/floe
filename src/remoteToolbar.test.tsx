import {expect,test} from 'vitest';
import {renderToStaticMarkup} from 'react-dom/server';
import {toolbarPull,pushTargetFor} from './remoteToolbar';
import {PushRemoteDialog} from './PushRemoteDialog';
import {demo} from './demo';
import type {Snapshot} from './types';
const remote={name:'work',fetchUrls:['https://example.invalid/fetch'],pushUrls:['https://example.invalid/push'],fetchRefspecs:[],branches:[{name:'release',oid:'a'.repeat(40),symbolicTarget:null},{name:'HEAD',oid:'a'.repeat(40),symbolicTarget:'refs/remotes/work/release'}]};
const snapshot:Snapshot={...demo,repoId:'fixture',revision:'frozen',headOid:'a'.repeat(40),detached:false,branch:'topic',branches:[{name:'topic',oid:'a'.repeat(40),current:true,upstream:{remotes:['work'],merges:['refs/heads/release']}}],remotes:[remote],pushRecord:null,pushRecordError:null};
test('toolbar Pull uses exact current upstream and never guesses origin or a sole remote',()=>{
 expect(toolbarPull(snapshot)).toEqual({kind:'pull',name:'work'});
 for(const upstream of [null,{remotes:['missing'],merges:['refs/heads/main']},{remotes:['work','other'],merges:['refs/heads/main']},{remotes:['work'],merges:['refs/heads/a','refs/heads/b']},{remotes:['work'],merges:['refs/tags/v1']}])expect(toolbarPull({...snapshot,branches:[{...snapshot.branches[0],upstream}]}).kind).toBe('configure');
 expect(toolbarPull({...snapshot,remotes:[]})).toEqual({kind:'addRemote'});
 for(const change of [{detached:true},{headOid:null},{repoId:'demo'},{branches:[]}])expect(toolbarPull({...snapshot,...change}).kind).toBe('unavailable');
});
test('push remote choice freezes source/revision and only includes nonsymbolic destination suggestions',()=>{
 expect(pushTargetFor(snapshot,'work')).toEqual({repoId:'fixture',revision:'frozen',name:'work',source:'topic',sourceOid:'a'.repeat(40),branches:['release']});
 expect(pushTargetFor(snapshot,'missing')).toBeNull();
 for(const change of [{detached:true},{headOid:null},{repoId:'demo'},{pushRecordError:'damaged'}])expect(pushTargetFor({...snapshot,...change},'work')).toBeNull();
});
test('push chooser requires explicit choice for multiple remotes and displays push URL preference',()=>{
 const props={source:'topic',busy:false,onChoose:()=>{},onClose:()=>{}};
 const one=renderToStaticMarkup(<PushRemoteDialog {...props} remotes={[remote]}/>);expect(one).toContain('https://example.invalid/push');expect(one).not.toContain('https://example.invalid/fetch');expect(one).not.toContain('disabled="">Choose destination');
 const many=renderToStaticMarkup(<PushRemoteDialog {...props} remotes={[remote,{...remote,name:'other'}]}/>);expect(many).toContain('disabled="">Choose destination');
 expect(renderToStaticMarkup(<PushRemoteDialog {...props} remotes={[]}/>)).toContain('Add a remote first');
 expect(renderToStaticMarkup(<PushRemoteDialog {...props} remotes={[remote]} busy/>)).toContain('disabled="">Choose destination');
});
