import {test,expect} from 'vitest';
import {renderToStaticMarkup} from 'react-dom/server';
import {PushResultBanner} from './PushResultBanner';
import type {PushRecord} from './types';
const record:PushRecord={id:'id',remote:'<origin>',endpoint:'https://example.com/r',source:'refs/heads/main',destination:'refs/heads/main',oldOid:null,newOid:'a'.repeat(40),status:'pending',detail:null};
const props={record,busy:false,onCheck:()=>{},onAcknowledge:()=>{}};
test('pending results require observation before archive and never offer retry',()=>{const html=renderToStaticMarkup(<PushResultBanner {...props}/>);expect(html).toContain('Check remote result');expect(html).not.toContain('Archive unconfirmed result');expect(html).toContain('No automatic retry');expect(html).toContain('&lt;origin&gt;');});
test('observed target remains explicitly uncertain and busy blocks actions',()=>{const html=renderToStaticMarkup(<PushResultBanner {...props} busy record={{...record,observation:{kind:'targetPresent',oid:record.newOid,checkedAt:100}}}/>);expect(html).toContain('does not prove whether the earlier Push succeeded');expect(html).toContain('disabled="">Archive unconfirmed result');expect(html).toContain('Push result unconfirmed');});
test('confirmed acceptance can close without remote reconciliation',()=>{const html=renderToStaticMarkup(<PushResultBanner {...props} record={{...record,status:'accepted'}}/>);expect(html).toContain('Server accepted Push');expect(html).toContain('Close confirmed Push result');expect(html).not.toContain('Check remote result');});
