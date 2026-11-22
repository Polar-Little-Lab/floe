import type {DiffCell,SplitRow} from './splitDiff';
export type AlignmentBudget={remaining:number};
/** Order-preserving similarity anchors; unmatched gaps retain positional pairing. */
export function alignDiffLines(left:DiffCell[],right:DiffCell[],budget:AlignmentBudget):SplitRow[]{
 const rows:SplitRow[]=[];
 const gap=(a:number,b:number,endA:number,endB:number)=>{
  while(a<endA||b<endB)rows.push({kind:'lines',left:a<endA?left[a++]:null,right:b<endB?right[b++]:null});
 };
 const cost=left.length*right.length;
 if(!cost||left.length>128||right.length>128||cost>budget.remaining||[...left,...right].some(cell=>cell.text.length>4096)){
  gap(0,0,left.length,right.length);return rows;
 }
 const tokenize=(cell:DiffCell)=>cell.text.match(/[\p{L}\p{N}_]+/gu)??[];
 const a=left.map(tokenize),b=right.map(tokenize);
 if([...a,...b].some(tokens=>tokens.length>128)){gap(0,0,left.length,right.length);return rows;}
 budget.remaining-=cost;
 const counts=b.map(tokens=>{const map=new Map<string,number>();for(const token of tokens)map.set(token,(map.get(token)??0)+1);return map;});
 const scores=new Uint16Array(cost);
 for(let i=0;i<left.length;i++)for(let j=0;j<right.length;j++){
  if(left[i].text===right[j].text){scores[i*right.length+j]=1000;continue;}
  if(!a[i].length||!b[j].length)continue;
  let common=0;const seen=new Map<string,number>();
  for(const token of a[i]){const count=(seen.get(token)??0)+1;seen.set(token,count);if(count<=(counts[j].get(token)??0))common++;}
  const similarity=2*common/(a[i].length+b[j].length);
  if(similarity>=0.5)scores[i*right.length+j]=Math.round(similarity*1000);
 }
 const width=right.length+1,table=new Uint32Array((left.length+1)*width);
 for(let i=left.length-1;i>=0;i--)for(let j=right.length-1;j>=0;j--){
  const score=scores[i*right.length+j];
  table[i*width+j]=Math.max(table[(i+1)*width+j],table[i*width+j+1],score?score+table[(i+1)*width+j+1]:0);
 }
 let i=0,j=0,startA=0,startB=0;
 while(i<left.length&&j<right.length){
  const score=scores[i*right.length+j];
  if(score&&table[i*width+j]===score+table[(i+1)*width+j+1]){
   gap(startA,startB,i,j);rows.push({kind:'lines',left:left[i],right:right[j]});startA=++i;startB=++j;
  }else if(table[(i+1)*width+j]>=table[i*width+j+1])i++;else j++;
 }
 gap(startA,startB,left.length,right.length);return rows;
}
