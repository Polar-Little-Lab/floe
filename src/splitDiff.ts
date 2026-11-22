import {alignDiffLines} from './alignDiffLines';
export interface DiffCell {number:number;text:string;kind:'context'|'add'|'remove';noNewline?:boolean}
export type SplitRow = {kind:'meta';text:string}|{kind:'lines';left:DiffCell|null;right:DiffCell|null};
export interface SplitResult {rows:SplitRow[];supported:boolean;limited:boolean;sourceLines:Map<DiffCell,number>}
// Align changed blocks using bounded similarity anchors; retain original cells and source positions.
export function splitDiff(text:string,truncated=false):SplitResult {
 const rows:SplitRow[]=[];const sourceLines=new Map<DiffCell,number>();
 let oldLine=0,newLine=0,oldLeft=0,newLeft=0,inHunk=false;
 let removed:DiffCell[]=[],added:DiffCell[]=[];const alignmentBudget={remaining:20_000};
 let last:DiffCell[]=[];
 const flush=()=>{rows.push(...alignDiffLines(removed,added,alignmentBudget));removed=[];added=[];};
 const failure=():SplitResult=>({rows:[],supported:false,limited:false,sourceLines:new Map()});
 const lines=text.split('\n');
 if(lines.at(-1)==='')lines.pop();
 for(const [sourceIndex,line] of lines.entries()){
  if(rows.length+removed.length+added.length>=10_000){flush();return {rows,supported:true,limited:true,sourceLines};}
  if(line==='\\ No newline at end of file'){
   if(!last.length)return failure();
   for(const cell of last)cell.noNewline=true;
   continue;
  }
  const hunk=/^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@(?:.*)$/.exec(line);
  if(hunk){
   if(inHunk&&(oldLeft||newLeft))return failure();
   flush();oldLine=Number(hunk[1]);newLine=Number(hunk[3]);oldLeft=Number(hunk[2]??1);newLeft=Number(hunk[4]??1);
   if(![oldLine,newLine,oldLeft,newLeft,oldLine+oldLeft,newLine+newLeft].every(Number.isSafeInteger))return failure();
   inHunk=true;last=[];rows.push({kind:'meta',text:line});continue;
  }
  if(line.startsWith('@@'))return failure(); // combined or malformed hunk
  if(inHunk&&(oldLeft||newLeft)){
   const body=line.slice(1);
   if(line.startsWith('-')&&oldLeft>0){const cell:DiffCell={number:oldLine++,text:body,kind:'remove'};sourceLines.set(cell,sourceIndex);removed.push(cell);oldLeft--;last=[cell];}
   else if(line.startsWith('+')&&newLeft>0){const cell:DiffCell={number:newLine++,text:body,kind:'add'};sourceLines.set(cell,sourceIndex);added.push(cell);newLeft--;last=[cell];}
   else if(line.startsWith(' ')&&oldLeft>0&&newLeft>0){flush();const left:DiffCell={number:oldLine++,text:body,kind:'context'},right:DiffCell={number:newLine++,text:body,kind:'context'};rows.push({kind:'lines',left,right});oldLeft--;newLeft--;last=[left,right];}
   else return failure();
  }else{
   // A body-looking line outside declared hunk bounds is never assigned invented numbers.
   if(inHunk&&!/^(diff --git |index |--- |\+\+\+ |new file |deleted file |old mode |new mode |similarity |dissimilarity |rename |copy |Binary files |GIT binary patch)/.test(line))return failure();
   flush();inHunk=false;last=[];rows.push({kind:'meta',text:line});
  }
 }
 if(inHunk&&(oldLeft||newLeft)&&!truncated)return failure();
 flush();return {rows,supported:true,limited:false,sourceLines};
}
