import { expect, test } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { FileContentView } from './FileContentView';
import type { FileContent } from './types';
test('full file preview preserves whitespace, escapes markup and discloses truncation',()=>{
 const content:FileContent={path:'x',source:'Index',state:'text',size:30,text:'<script>bad</script>\n\nlast',truncated:false,endsWithNewline:false};
 const html=renderToStaticMarkup(<FileContentView content={content}/>);
 expect(html).not.toContain('<script>');expect(html).toContain('&lt;script&gt;');
 expect(html.match(/class="code-line"/g)).toHaveLength(3);expect(html).toContain('No newline at end of file');
 const truncated=renderToStaticMarkup(<FileContentView content={{...content,truncated:true}}/>);
 expect(truncated).toContain('Preview truncated');expect(truncated).not.toContain('No newline at end of file');
 const binary=renderToStaticMarkup(<FileContentView content={{...content,state:'binary',text:''}}/>);
 expect(binary).toContain('Binary file');expect(binary).not.toContain('Empty file');
});
