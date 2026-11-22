import { expect,test } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { AmendControl } from './AmendControl';
test('amend requires a writable HEAD and explains replacement and author preservation',()=>{
 for(const [writable,hasHead] of [[false,true],[true,false]]){
  expect(renderToStaticMarkup(<AmendControl checked={false} writable={writable} hasHead={hasHead} onChange={()=>{}}/>)).toContain('disabled=""');
 }
 const html=renderToStaticMarkup(<AmendControl checked writable hasHead onChange={()=>{}}/>);
 expect(html).toContain('checked=""');expect(html).not.toContain('disabled=""');
 expect(html).toContain('Replaces current HEAD');expect(html).toContain('Original author and parents are preserved');
});
