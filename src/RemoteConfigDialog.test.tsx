import {expect,test} from 'vitest';
import {renderToStaticMarkup} from 'react-dom/server';
import {RemoteConfigDialog} from './RemoteConfigDialog';
test('remote editor does not reuse redacted credentials and discloses endpoint replacement',()=>{
 const html=renderToStaticMarkup(<RemoteConfigDialog existing="origin" busy={false} error="" onClose={()=>{}} onSave={()=>{}}/>);
 expect(html).toContain('Saving replaces all existing fetch and push URLs');
 expect(html).toContain('Do not enter passwords or tokens');
 expect(html).toContain('aria-label="Remote name" disabled=""');
 expect(html).toContain('value="origin"');
 expect(html).not.toContain('[redacted]');
 expect(html).toContain('disabled="">Save remote');
});
