import {expect,test} from 'vitest';
import {splitDiff} from './splitDiff';
import {wordDiff} from './wordDiff';
test('highlights separate edits while preserving unchanged words, punctuation and exact Unicode text',()=>{
 const parsed=splitDiff('@@ -1 +1 @@\n-const 이름 = old + first; 😀\n+const 이름 = newer + last; 😀\n');
 const result=wordDiff(parsed.rows);expect(result.limited).toBe(false);
 const values=[...result.cells.entries()];expect(values).toHaveLength(2);
 for(const [cell,parts] of values)expect(parts.map(part=>part.text).join('')).toBe(cell.text);
 expect(values[0][1].filter(p=>p.changed).map(p=>p.text)).toEqual(['old','first']);
 expect(values[1][1].filter(p=>p.changed).map(p=>p.text)).toEqual(['newer','last']);
});
test('handles whitespace-only and empty-line edits without treating context or unpaired rows as replacements',()=>{
 const parsed=splitDiff('@@ -1,3 +1,4 @@\n same\n-a b\n-\n+a  b\n+x\n+extra\n');const result=wordDiff(parsed.rows);
 expect(result.cells.size).toBe(4);
 expect([...result.cells.values()].some(parts=>parts.some(part=>part.changed&&part.text==='  '))).toBe(true);
 expect([...result.cells.keys()].every(cell=>cell.kind!=='context'&&cell.text!=='extra')).toBe(true);
});
test('uses a shared comparison budget and skips long lines without changing the parsed patch',()=>{
 const parsed=splitDiff('@@ -1,2 +1,2 @@\n-a\n-c\n+b\n+d\n');const before=JSON.stringify(parsed);
 const result=wordDiff(parsed.rows,4);expect(result.cells.size).toBe(2);expect(result.limited).toBe(true);expect(JSON.stringify(parsed)).toBe(before);
 const long=splitDiff(`@@ -1 +1 @@\n-${'x'.repeat(4097)}\n+y\n`);expect(wordDiff(long.rows)).toEqual({cells:new Map(),limited:true});
});

test('source line mapping keeps repeated text in separate hunks and files distinct',()=>{
 const text='diff --git a/x b/x\n@@ -1 +1 @@\n-old\n\\ No newline at end of file\n+new\n@@ -9 +9 @@\n-old\n+last\ndiff --git a/y b/y\n@@ -1 +1 @@\n-old\n+other\n';
 const parsed=splitDiff(text);expect(parsed.supported).toBe(true);
 const words=wordDiff(parsed.rows);const lines=text.split('\n');
 const indices=[...words.cells].map(([cell,parts])=>{
  const index=parsed.sourceLines.get(cell)!;
  expect(lines[index]).toBe((cell.kind==='remove'?'-':'+')+parts.map(part=>part.text).join(''));
  return index;
 });
 expect(indices).toEqual([2,4,6,7,10,11]);
});
