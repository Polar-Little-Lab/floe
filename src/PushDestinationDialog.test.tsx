import {expect,test} from 'vitest';
import {renderToStaticMarkup} from 'react-dom/server';
import {PushDestinationDialog} from './PushDestinationDialog';
const props={name:'<origin>',source:'main',sourceOid:'a'.repeat(40),branches:['main','release/v2','release/v2'],busy:false,error:'',onClose:()=>{},onPreview:()=>{}};
test('push destination shows frozen source and fetched suggestions without sending commits',()=>{
 const html=renderToStaticMarkup(<PushDestinationDialog {...props}/>);
 expect(html).toContain('Push to &lt;origin&gt;');expect(html).toContain('Source: main');expect(html).toContain('aaaaaaaaaaaa');expect(html).toContain('value="main"');expect(html.match(/<option value="release\/v2"/g)).toHaveLength(1);expect(html).toContain('Preview sends no commits.');expect(html).toContain('upstream settings stay unchanged');
});
test('push destination blocks empty input and disables all controls while busy',()=>{
 expect(renderToStaticMarkup(<PushDestinationDialog {...props} source=""/>)).toContain('disabled="">Preview destination');
 const html=renderToStaticMarkup(<PushDestinationDialog {...props} busy error="STALE_STATE"/>);
 expect(html).toContain('role="alert">STALE_STATE');expect(html.match(/disabled=""/g)).toHaveLength(4);expect(html).toContain('Checking destination…');
});
