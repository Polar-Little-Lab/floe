import {expect,it} from 'vitest';
import {renderToStaticMarkup} from 'react-dom/server';
import {CommitCompareDialog} from './CommitCompareDialog';
const commits=[{oid:'a'.repeat(40),parents:[],summary:'Base',author:'Test',time:0,refs:[]},{oid:'b'.repeat(40),parents:[],summary:'Target',author:'Test',time:0,refs:[]}];
const props={commits,initialBase:commits[0].oid,initialTarget:commits[1].oid,selected:null,busy:false,error:'',onCompare:()=>{},onReset:()=>{},onClose:()=>{}};
it('never displays a patch returned for a different comparison direction',()=>{
 const html=renderToStaticMarkup(<CommitCompareDialog {...props} result={{baseOid:commits[1].oid,targetOid:commits[0].oid,files:[{id:'file',path:'wrong direction',status:'added',staged:false,unstaged:false}],diff:null}}/>);
 expect(html).not.toContain('wrong direction');expect(html).toContain('Direct comparison');
});
it('shows empty comparisons explicitly and blocks controls during a pending request',()=>{
 const html=renderToStaticMarkup(<CommitCompareDialog {...props} busy result={{baseOid:commits[0].oid,targetOid:commits[1].oid,files:[],diff:null}}/>);
 expect(html).toContain('No file differences');expect(html).toContain('Loading comparison');
 for(const control of html.match(/<(?:button|select)\b[^>]*>/g)??[])expect(control).toContain('disabled=""');
});
