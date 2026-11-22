import {test,expect} from 'vitest';
import {renderToStaticMarkup} from 'react-dom/server';
import {RenameBranchDialog} from './RenameBranchDialog';
test('rename cannot submit unchanged name or during operation and escapes untrusted text',()=>{
 const props={name:'topic',value:'topic',busy:false,error:'',onChange:()=>{},onClose:()=>{},onSubmit:()=>{}};
 expect(renderToStaticMarkup(<RenameBranchDialog {...props}/>)).toContain('disabled=""');
 const pending=renderToStaticMarkup(<RenameBranchDialog {...props} value="new" busy/>);
 expect(pending.match(/disabled=""/g)).toHaveLength(3);expect(pending).toContain('Renaming…');
 const html=renderToStaticMarkup(<RenameBranchDialog {...props} value="new" error="<bad>"/>);
 expect(html).toContain('role="alert"');expect(html).toContain('&lt;bad&gt;');expect(html).not.toContain('disabled=""');expect(html).toContain('autoCorrect="off"');expect(html).toContain('autoCapitalize="none"');expect(html).toContain('spellCheck="false"');
});
