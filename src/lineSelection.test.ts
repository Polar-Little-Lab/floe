import {expect,test} from 'vitest';
import {splitDiff} from './splitDiff';
import {selectableDiffRows} from './lineSelection';
import type {DiffResult} from './types';
import {renderToStaticMarkup} from 'react-dom/server';
import {createElement} from 'react';
import {DiffView} from './DiffView';
const diff:DiffResult={text:'diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1,2 +1,3 @@\n-const a = old;\n-const b = before;\n+log startup\n+const a = new;\n+const b = after;\n',truncated:false,selectableLines:[{hunk:0,line:0,oldLine:1,newLine:null},{hunk:0,line:1,oldLine:2,newLine:null},{hunk:0,line:2,oldLine:null,newLine:1},{hunk:0,line:3,oldLine:null,newLine:2},{hunk:0,line:4,oldLine:null,newLine:3}]};
test('selection IDs follow original patch rows despite similarity alignment',()=>{
 const rows=selectableDiffRows(diff,splitDiff(diff.text));
 expect([...rows].sort(([a],[b])=>a-b)).toEqual(diff.selectableLines!.map((line,i)=>[i+4,line]));
});
test('truncated, unsupported or read-only metadata never invent selectable lines',()=>{
 expect(selectableDiffRows({...diff,truncated:true},splitDiff(diff.text)).size).toBe(0);
 expect(selectableDiffRows(diff,splitDiff('@@@ combined')) .size).toBe(0);
 expect(selectableDiffRows({...diff,selectableLines:[]},splitDiff(diff.text)).size).toBe(0);
});
test('both layouts expose only changed lines and guard busy/read-only actions',()=>{
 for(const initialMode of ['unified','split'] as const){
  const props={diff,busy:true,initialMode,hunkAction:'Stage' as const,onLines:()=>{}};
  const html=renderToStaticMarkup(createElement(DiffView,props));
  expect((html.match(/aria-label="Select diff line/g)??[]).length).toBe(5);
  expect((html.match(/type="checkbox" disabled=""/g)??[]).length).toBe(5);
  expect(renderToStaticMarkup(createElement(DiffView,{...props,onLines:undefined}))).not.toContain('Select diff line');
 }
});

import {selectableRowOrder,toggleLineRange} from './lineSelection';
test('split ranges use visible before/after order rather than original patch order',()=>{
 const parsed=splitDiff(diff.text),map=selectableDiffRows(diff,parsed);
 expect(selectableRowOrder(map,null)).toEqual([4,5,6,7,8]);
 const order=selectableRowOrder(map,parsed);
 expect(order).toEqual([6,4,7,5,8]);
 expect([...toggleLineRange(new Set([4]),order,4,7,true)]).toEqual([4,7]);
 expect([...toggleLineRange(new Set([4]),selectableRowOrder(map,null),4,7,true)]).toEqual([4,5,6,7]);
});
test('range deselection, backward range, missing anchor and invalid endpoint remain bounded',()=>{
 const order=[6,4,7,5,8];
 expect([...toggleLineRange(new Set([8]),order,8,4,true)].sort()).toEqual([4,5,7,8]);
 expect([...toggleLineRange(new Set([4,5,7,8]),order,8,4,true)]).toEqual([]);
 expect([...toggleLineRange(new Set(),order,999,7,true)]).toEqual([7]);
 const previous=new Set([4]);expect(toggleLineRange(previous,order,4,999,true)).toBe(previous);
});
