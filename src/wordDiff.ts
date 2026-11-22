import type {SplitRow,DiffCell} from './splitDiff';
export type WordPart={text:string;changed:boolean};
const tokenize=(text:string)=>text.match(/[\p{L}\p{N}_]+|\s+|[^\p{L}\p{N}_\s]/gu)??[];
function parts(tokens:string[],same:Set<number>):WordPart[]{
 const result:WordPart[]=[];
 tokens.forEach((text,index)=>{const changed=!same.has(index);const last=result.at(-1);if(last?.changed===changed)last.text+=text;else result.push({text,changed});});
 return result;
}
// A shared matrix-cell budget bounds work across all paired lines in a displayed patch.
export function wordDiff(rows:SplitRow[],budget=250_000):{cells:Map<DiffCell,WordPart[]>;limited:boolean}{
 const cells=new Map<DiffCell,WordPart[]>();let limited=false;
 for(const row of rows){
  if(row.kind!=='lines'||row.left?.kind!=='remove'||row.right?.kind!=='add')continue;
  const left=row.left,right=row.right;if(left.text===right.text)continue;
  if(left.text.length>4096||right.text.length>4096){limited=true;continue;}
  const a=tokenize(left.text),b=tokenize(right.text);const cost=(a.length+1)*(b.length+1);
  if(a.length>256||b.length>256||cost>budget){limited=true;continue;}budget-=cost;
  const width=b.length+1;const table=new Uint16Array(cost);
  for(let i=a.length-1;i>=0;i--)for(let j=b.length-1;j>=0;j--)table[i*width+j]=a[i]===b[j]?1+table[(i+1)*width+j+1]:Math.max(table[(i+1)*width+j],table[i*width+j+1]);
  const sameA=new Set<number>(),sameB=new Set<number>();let i=0,j=0;
  while(i<a.length&&j<b.length){if(a[i]===b[j]){sameA.add(i++);sameB.add(j++);}else if(table[(i+1)*width+j]>=table[i*width+j+1])i++;else j++;}
  cells.set(left,parts(a,sameA));cells.set(right,parts(b,sameB));
 }
 return {cells,limited};
}
