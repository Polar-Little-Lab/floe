import { expect,test } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { OpenRepositoryDialog } from './OpenRepositoryDialog';
test('path dialog preserves literal path, exposes errors and disables actions while opening',()=>{
 const props={path:'/tmp/repo & spaces',busy:false,error:'NOT_REPOSITORY: <bad>',onPath:()=>{},onClose:()=>{},onBrowse:()=>{},onOpen:()=>{}};
 const html=renderToStaticMarkup(<OpenRepositoryDialog {...props}/>);
 expect(html).toContain('/tmp/repo &amp; spaces');expect(html).toContain('role="alert"');expect(html).not.toContain('<bad>');expect(html).toContain('Browse folders');expect(html).toContain('Open path');
 const busy=renderToStaticMarkup(<OpenRepositoryDialog {...props} busy/>);
 expect(busy.match(/disabled=""/g)).toHaveLength(4);expect(busy).toContain('Opening…');
});

test('long wait keeps repository controls locked and explains pending system dialogs',()=>{
 const props={path:'/tmp/repo',busy:true,waitingLong:true,error:'',onPath:()=>{},onClose:()=>{},onBrowse:()=>{},onOpen:()=>{}};
 const html=renderToStaticMarkup(<OpenRepositoryDialog {...props}/>);
 expect(html).toContain('role="status"');expect(html).toContain('macOS');expect(html).toContain('아직 완료되거나 취소된 상태가 아닙니다.');
 expect(html.match(/disabled=""/g)).toHaveLength(4);
 expect(renderToStaticMarkup(<OpenRepositoryDialog {...props} busy={false}/>)).not.toContain('role="status"');
});

test('native create entry is available and cannot switch modes during an operation',()=>{
 const props={path:'/tmp/new',busy:true,error:'',onPath:()=>{},onClose:()=>{},onBrowse:()=>{},onOpen:()=>{},onCreate:()=>{}};
 const html=renderToStaticMarkup(<OpenRepositoryDialog {...props}/>);
 expect(html).toContain('Create new');
 expect(html).toContain('disabled="" aria-pressed="false"');
 expect(html.match(/disabled=""/g)).toHaveLength(6);
});

test('clone mode presents HTTPS scope and native credential entry, optional branch and protects controls while busy',()=>{
 const props={path:'/tmp/clone',busy:true,error:'',initialMode:'clone' as const,onPath:()=>{},onClose:()=>{},onBrowse:()=>{},onOpen:()=>{},onCreate:()=>{},onClone:()=>{}};
 const html=renderToStaticMarkup(<OpenRepositoryDialog {...props}/>);
 expect(html).toContain('Clone URL');expect(html).toContain('Clone branch (optional)');expect(html).toContain('HTTPS repositories.');expect(html).toContain('On macOS, enter credentials in the native confirmation.');expect(html).not.toContain('type="password"');expect(html).toContain('Cloning…');
 expect(html).not.toContain('Browse folders');expect(html).not.toContain('Initial branch');
 expect(html.match(/disabled=""/g)).toHaveLength(8);
 expect(renderToStaticMarkup(<OpenRepositoryDialog {...props} busy={false}/>)).toContain('class="primary" disabled=""');
});
