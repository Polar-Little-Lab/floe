import { expect, test } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { StashPanel } from './StashPanel';
import type { StashDetail } from './types';

test('stash paths shared by staged and unstaged areas retain independent selection and escaped text', () => {
 const file={id:'same-path',path:'<img src=x onerror=alert(1)>.txt',status:'modified',staged:false,unstaged:false};
 const detail:StashDetail={oid:'stash',baseOid:'base',message:'<script>alert(1)</script>',diff:null,groups:[{area:'staged',files:[file]},{area:'unstaged',files:[file]},{area:'untracked',files:[]}]};
 const html=renderToStaticMarkup(<StashPanel detail={detail} busy={false} selected={{id:file.id,area:'unstaged'}} onClose={()=>{}} onFile={()=>{}}/>);
 expect(html.match(/history-file chosen/g)).toHaveLength(1);
 expect(html).toContain('Staged changes (1)');
 expect(html).toContain('Unstaged changes (1)');
 expect(html).toContain('Untracked files (0)');
 expect(html).not.toContain('<script>');
 expect(html).not.toContain('<img');
 expect(html).toContain('&lt;script&gt;');
});
