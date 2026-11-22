import { expect,test } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { DiffView } from './DiffView';
test('literal diff search opts out of native spelling and capitalization changes',()=>{
 const html=renderToStaticMarkup(<DiffView busy={false} diff={null}/>);
 const input=html.match(/<input[^>]*aria-label="Find in displayed diff"[^>]*>/)?.[0];
 expect(input).toContain('autoCorrect="off"');
 expect(input).toContain('autoCapitalize="none"');
 expect(input).toContain('spellCheck="false"');
});
test('split rendering escapes repository text and labels before/after columns',()=>{
 const html=renderToStaticMarkup(<DiffView busy={false} initialMode="split" diff={{text:'@@ -1 +1 @@\n-<script>old</script>\n+<img src=x>\n',truncated:false}}/>);
 expect(html).toContain('Before');expect(html).toContain('After');expect(html).toContain('split-cell remove');expect(html).toContain('split-cell add');
 expect(html).not.toContain('<script>');expect(html).not.toContain('<img');expect(html.replace(/<span class="diff-word-change">|<\/span>/g,'' )).toContain('&lt;script&gt;old&lt;/script&gt;');expect(html).toContain('diff-word-change');
});
test('unsupported split formats retain the original patch with a clear fallback notice',()=>{
 const html=renderToStaticMarkup(<DiffView busy={false} initialMode="split" diff={{text:'@@@ combined @@@\n',truncated:false}}/>);
 expect(html).toContain('Showing unified diff');expect(html).toContain('@@@ combined @@@');
});

test('unified word changes preserve original order, prefixes, metadata and missing-newline markers',()=>{
 const text='diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1,2 +1,2 @@\n-same old\n-<script>before</script>\n+same new\n+<script>after</script>\n\\ No newline at end of file\n';
 const html=renderToStaticMarkup(<DiffView busy={false} initialMode="unified" diff={{text,truncated:false}}/>);
 const code=[...html.matchAll(/<code>([\s\S]*?)<\/code>/g)].map(match=>match[1]);
 expect(code[1]).toBe('--- a/x');expect(code[2]).toBe('+++ b/x');
 expect(code[4]).toBe('-same <span class="diff-word-change">old</span>');
 expect(code[6]).toBe('+same <span class="diff-word-change">new</span>');
 expect(code[5]).toContain('&lt;script&gt;');expect(html).not.toContain('<script>');
 expect(code[8]).toBe('\\ No newline at end of file');
 expect(html).not.toContain('split-labels');
});

test('unified malformed patches and over-budget lines keep original text',()=>{
 for(const text of ['@@ -1 +1 @@\n-old\n+new\n+extra\n',`@@ -1 +1 @@\n-${'x'.repeat(4097)}\n+y\n`]){
  const html=renderToStaticMarkup(<DiffView busy={false} initialMode="unified" diff={{text,truncated:false}}/>);
  expect(html).not.toContain('class="diff-word-change"');
  expect(html).toContain(text.includes('extra')?'+extra':'x'.repeat(4097));
 }
});

test('unified rendering retains lines beyond alignment limit and announces limited highlighting',()=>{
 const text='@@ -0,0 +1,10002 @@\n'+Array.from({length:10002},(_,index)=>`+line ${index}`).join('\n')+'\n';
 const html=renderToStaticMarkup(<DiffView busy={false} initialMode="unified" diff={{text,truncated:false}}/>);
 expect(html).toContain('+line 10001');
 expect(html).toContain('Word highlighting is limited');
 expect(html).not.toContain('Split preview limited');
});

test('stage and unstage controls use backend hunk indices in unified and split views',()=>{
 const diff={text:'@@ -1 +1 @@\n-old\n+new\n',truncated:false,hunks:[{index:0,header:'@@ -1 +1 @@'}]};
 for(const mode of ['unified','split'] as const){const html=renderToStaticMarkup(<DiffView diff={diff} busy initialMode={mode} hunkAction="Unstage" onHunk={()=>{}}/>);expect(html).toContain('aria-label="Unstage hunk 1"');expect(html).toContain('class="hunk-action" disabled=""');}
 expect(renderToStaticMarkup(<DiffView diff={diff} busy={false}/>)).not.toContain('hunk-action');
 expect(renderToStaticMarkup(<DiffView diff={{...diff,truncated:true}} busy={false} hunkAction="Stage" onHunk={()=>{}}/>)).not.toContain('hunk-action');
});
