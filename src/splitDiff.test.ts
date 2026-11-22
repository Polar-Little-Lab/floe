import { describe,expect,test } from 'vitest';
import { splitDiff } from './splitDiff';
describe('split diff alignment',()=>{
 test('keeps context and unequal replacement blocks aligned with actual line numbers',()=>{
  const p=splitDiff('@@ -8,4 +20,5 @@ function\n context\n-old one\n-old two\n+new one\n+new two\n+new three\n tail\n');
  expect(p.supported).toBe(true);
  expect(p.rows.filter(r=>r.kind==='lines')).toEqual([
   {kind:'lines',left:{number:8,text:'context',kind:'context'},right:{number:20,text:'context',kind:'context'}},
   {kind:'lines',left:{number:9,text:'old one',kind:'remove'},right:{number:21,text:'new one',kind:'add'}},
   {kind:'lines',left:{number:10,text:'old two',kind:'remove'},right:{number:22,text:'new two',kind:'add'}},
   {kind:'lines',left:null,right:{number:23,text:'new three',kind:'add'}},
   {kind:'lines',left:{number:11,text:'tail',kind:'context'},right:{number:24,text:'tail',kind:'context'}},
  ]);
 });
 test('handles zero-length ranges, omitted counts, repeated hunks and header-looking content',()=>{
  const p=splitDiff('diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -0,0 +1,2 @@\n+first\n+second\n@@ -10 +12,0 @@\n--- removed content\n');
  expect(p.supported).toBe(true);
  expect(p.rows.at(-1)).toEqual({kind:'lines',left:{number:10,text:'-- removed content',kind:'remove'},right:null});
  expect(p.rows.filter(r=>r.kind==='lines')).toHaveLength(3);
 });
 test('marks missing final newlines on each corresponding side without shifting numbers',()=>{
  const p=splitDiff('@@ -1 +1 @@\n-old\n\\ No newline at end of file\n+new\n\\ No newline at end of file\n');
  expect(p.rows[1]).toEqual({kind:'lines',left:{number:1,text:'old',kind:'remove',noNewline:true},right:{number:1,text:'new',kind:'add',noNewline:true}});
 });
 test('retains rename and binary metadata without fabricating code rows',()=>{
  for(const patch of ['similarity index 100%\nrename from old\nrename to new\n','Binary files a/image and b/image differ\n']){
   const p=splitDiff(patch);expect(p.supported).toBe(true);expect(p.rows.every(r=>r.kind==='meta')).toBe(true);
  }
 });
 test('falls back for malformed, combined, unsafe-number and incomplete hunks',()=>{
  for(const patch of ['@@@ -1,1 -1,1 +1,1 @@@\n','@@ -1 +1 @@\n+new\n','@@ -1 +1 @@\n-old\n+new\n+extra\n','@@ -9007199254740992 +1 @@\n-x\n+y\n'])expect(splitDiff(patch).supported).toBe(false);
  expect(splitDiff('@@ -1 +1 @@\n-old\n',true).supported).toBe(true);
 });
 test('bounds large displays and preserves empty lines and Unicode',()=>{
  const p=splitDiff('@@ -0,0 +1,10002 @@\n'+Array.from({length:10002},(_,i)=>i===0?'+':'+한글').join('\n'));
  expect(p.supported).toBe(true);expect(p.limited).toBe(true);expect(p.rows.length).toBeLessThanOrEqual(10000);
  expect(p.rows[1]).toEqual({kind:'lines',left:null,right:{number:1,text:'',kind:'add'}});
 });
});
