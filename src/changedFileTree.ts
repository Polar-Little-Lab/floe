import type {Change} from './types';
type FolderNode={kind:'folder';path:string;name:string;depth:number;count:number};
type FileNode={kind:'file';change:Change;depth:number};
type Node=FolderNode|FileNode;
export function makeTree(files:Change[]):Node[]{
 const roots:Node[]=[];const children=new Map<string,Node[]>([['',roots]]);const folders=new Map<string,FolderNode>();
 for(const change of files){const parts=change.path.split('/');let parent='';
  for(let depth=0;depth<parts.length-1;depth++){const path=parts.slice(0,depth+1).join('/');let folder=folders.get(path);
   if(!folder){folder={kind:'folder',path,name:parts[depth],depth,count:0};folders.set(path,folder);children.get(parent)!.push(folder);children.set(path,[]);}folder.count++;parent=path;
  }
  children.get(parent)!.push({kind:'file',change,depth:parts.length-1});
 }
 const result:Node[]=[];const stack=[...roots].reverse();while(stack.length){const node=stack.pop()!;result.push(node);if(node.kind==='folder')stack.push(...[...children.get(node.path)!].reverse());}return result;
}
