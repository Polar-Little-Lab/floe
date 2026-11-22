import {expect,test} from 'vitest';
import {alignDiffLines} from './alignDiffLines';
import {splitDiff,type DiffCell} from './splitDiff';
const cells=(texts:string[],kind:'add'|'remove')=>texts.map((text,i):DiffCell=>({text,kind,number:i+1}));
test('inserted leading line does not shift corresponding edits',()=>{
 const left=cells(['const first = old;','const second = before;'],'remove');
 const right=cells(['log startup','const first = new;','const second = after;'],'add');
 expect(alignDiffLines(left,right,{remaining:100})).toEqual([
  {kind:'lines',left:null,right:right[0]},
  {kind:'lines',left:left[0],right:right[1]},
  {kind:'lines',left:left[1],right:right[2]},
 ]);
});
test('deleted middle line stays separate from later corresponding edits',()=>{
 const left=cells(['alpha = old','remove entirely','beta = old'],'remove');
 const right=cells(['alpha = new','beta = new'],'add');
 const rows=alignDiffLines(left,right,{remaining:100});
 expect(rows).toEqual([{kind:'lines',left:left[0],right:right[0]},{kind:'lines',left:left[1],right:null},{kind:'lines',left:left[2],right:right[1]}]);
});
test('fallback preserves order, exact object identity and newline metadata',()=>{
 const left=cells(['이름 옛값','repeat','repeat'],'remove');left[2].noNewline=true;
 const right=cells(['이름 새값','repeat','extra','repeat'],'add');
 for(const remaining of [0,100]){
  const rows=alignDiffLines(left,right,{remaining});
  const actualLeft=rows.flatMap(row=>row.kind==='lines'&&row.left?[row.left]:[]);
  const actualRight=rows.flatMap(row=>row.kind==='lines'&&row.right?[row.right]:[]);
  expect(actualLeft).toEqual(left);expect(actualRight).toEqual(right);
  actualLeft.forEach((cell,i)=>expect(cell).toBe(left[i]));
  actualRight.forEach((cell,i)=>expect(cell).toBe(right[i]));
 }
});
test('shared budget and oversized blocks use positional fallback',()=>{
 const left=cells(['a old','b old'],'remove'),right=cells(['added','a new','b new'],'add');
 const budget={remaining:6};alignDiffLines(left,right,budget);expect(budget.remaining).toBe(0);
 expect(alignDiffLines(left,right,budget)[0]).toEqual({kind:'lines',left:left[0],right:right[0]});
 const long=cells(['x'.repeat(4097)],'remove');
 expect(alignDiffLines(long,right,{remaining:100})).toHaveLength(3);
});
test('parser alignment retains original unified source indices',()=>{
 const text='@@ -1,2 +1,3 @@\n-alpha = old\n-beta = old\n+inserted\n+alpha = new\n+beta = new\n';
 const parsed=splitDiff(text);expect(parsed.supported).toBe(true);
 const rows=parsed.rows.filter(row=>row.kind==='lines');
 expect(rows[0].left).toBeNull();expect(rows[0].right?.text).toBe('inserted');
 for(const row of rows)for(const cell of [row.left,row.right])if(cell){
  expect(text.split('\n')[parsed.sourceLines.get(cell)!]).toBe((cell.kind==='remove'?'-':'+')+cell.text);
 }
});
