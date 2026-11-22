import {it,expect} from 'vitest';
import {renderToStaticMarkup} from 'react-dom/server';
import {GraphPager} from './GraphPager';
it('shows real history range and explains page-scoped search at the final page',()=>{
 const html=renderToStaticMarkup(<GraphPager offset={1000} count={201} hasMore={false} busy={false} filtered onPage={()=>{}}/>);
 expect(html).toContain('1001–1201');expect(html).toContain('Search applies to this page');expect(html).toMatch(/<button disabled="">Older/);expect(html).toContain('<button>Latest');
});
it('prevents paging during a pending operation and before the first page',()=>{
 for(const busy of [false,true]){const html=renderToStaticMarkup(<GraphPager offset={0} count={500} hasMore busy={busy} filtered={false} onPage={()=>{}}/>);expect(html).toMatch(/<button disabled="">Latest/);expect(html).toMatch(/<button disabled="">Newer/);if(busy)expect(html).toMatch(/<button disabled="">Older/);else expect(html).toContain('<button>Older');}
});
