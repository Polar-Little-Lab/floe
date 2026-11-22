import type {DiffResult,SelectedDiffLine} from './types';
import type {SplitResult} from './splitDiff';
/** Use original side line numbers, never visual alignment positions or patch text supplied by UI. */
export function selectableDiffRows(diff:DiffResult|null,parsed:SplitResult|null):Map<number,SelectedDiffLine>{
 const result=new Map<number,SelectedDiffLine>();
 if(!diff||diff.truncated||!parsed?.supported)return result;
 const bySide=new Map((diff.selectableLines??[]).map(line=>[line.oldLine!==null?`remove:${line.oldLine}`:`add:${line.newLine}`,line]));
 for(const [cell,row] of parsed.sourceLines){const line=bySide.get(`${cell.kind}:${cell.number}`);if(line)result.set(row,line);}
 return result;
}

/** Split order is top-to-bottom, before then after within a displayed row. */
export function selectableRowOrder(selectable:Map<number,SelectedDiffLine>,split:SplitResult|null):number[]{
 if(!split?.supported)return [...selectable.keys()].sort((a,b)=>a-b);
 const result:number[]=[];
 for(const row of split.rows)if(row.kind==='lines')for(const cell of [row.left,row.right]){
  const source=cell?split.sourceLines.get(cell):undefined;
  if(source!==undefined&&selectable.has(source))result.push(source);
 }
 return result;
}
export function toggleLineRange(previous:Set<number>,order:number[],anchor:number|null,endpoint:number,range:boolean):Set<number>{
 const end=order.indexOf(endpoint);if(end<0)return previous;
 const start=anchor===null?-1:order.indexOf(anchor);
 const targets=range&&start>=0?order.slice(Math.min(start,end),Math.max(start,end)+1):[endpoint];
 const next=new Set(previous),select=!previous.has(endpoint);
 for(const row of targets)if(select)next.add(row);else next.delete(row);
 return next;
}
