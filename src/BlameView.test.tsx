import { expect, test } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { BlameView } from './BlameView';
import type { FileBlame } from './types';

test('blame keeps line attribution aligned across hunks and escapes file and author content',()=>{
 const blame:FileBlame={oid:'selected',path:'file.txt',historyLimited:true,lines:['<script>bad</script>','second','third'],hunks:[{start:1,lines:2,oid:'1111111',author:'<img src=x>',time:0,originalPath:'old.txt',originalStart:4,boundary:true},{start:3,lines:1,oid:'2222222',author:'Bob',time:1,originalPath:'file.txt',originalStart:3,boundary:false}]};
 const html=renderToStaticMarkup(<BlameView blame={blame}/>);
 expect(html.match(/<span>1111111<\/span>/g)).toHaveLength(2);
 expect(html.match(/<span>2222222<\/span>/g)).toHaveLength(1);
 expect(html).toContain('old.txt:5');
 expect(html).toContain('boundary attribution may be incomplete');
 expect(html).not.toContain('<script>');
 expect(html).not.toContain('<img');
 expect(html).toContain('&lt;script&gt;');
});
