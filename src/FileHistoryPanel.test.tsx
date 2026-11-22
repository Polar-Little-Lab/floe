import { expect, test } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { FileHistoryPanel } from './FileHistoryPanel';
import type { FileHistory } from './types';

test('last history page reports actual scan range and escapes repository text', () => {
 const page:FileHistory={path:'<img src=x>.txt',offset:100,scanned:3,nextOffset:null,limitReached:false,diff:null,entries:[{oid:'abc1234',summary:'<script>bad</script>',author:'Test',time:0,path:'new.txt',previousPath:'old.txt',status:'renamed'}]};
 const html=renderToStaticMarkup(<FileHistoryPanel page={page} busy={false} selected="abc1234" onPage={()=>{}} onSelect={()=>{}} onClose={()=>{}}/>);
 expect(html).toContain('101–103');
 expect(html).toContain('old.txt → new.txt');
 expect(html).toContain('history-file chosen');
 expect(html).toContain('disabled="">Next');
 expect(html).not.toContain('<script>');
 expect(html).not.toContain('<img');
});
