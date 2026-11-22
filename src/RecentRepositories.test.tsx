import { expect,test } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { RecentRepositories } from './RecentRepositories';
test('recent repository paths remain escaped literals and removal is list-only',()=>{
 const html=renderToStaticMarkup(<RecentRepositories paths={['/tmp/<script>','C:\\work\\project']} busy={false} onOpen={()=>{}} onForget={()=>{}}/>);
 expect(html).toContain('/tmp/&lt;script&gt;');expect(html).not.toContain('<script>');expect(html).toContain('C:\\work\\project');expect(html).toContain('Remove from this list only');
});
test('busy locks both opening and forgetting and empty list has no controls',()=>{
 expect(renderToStaticMarkup(<RecentRepositories paths={['/tmp/a','/tmp/b']} busy onOpen={()=>{}} onForget={()=>{}}/>).match(/disabled=""/g)).toHaveLength(4);
 expect(renderToStaticMarkup(<RecentRepositories paths={[]} busy={false} onOpen={()=>{}} onForget={()=>{}}/>)).toBe('');
});
