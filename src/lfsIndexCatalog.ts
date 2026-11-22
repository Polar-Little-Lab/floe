import {inspectLfsPointer} from './lfsPointer';
import type {LfsIndexPage} from './lfsTransfer';
export interface LfsIndexObject{oid:string;size:string;path:string;text:string;paths:string[]}
export interface LfsIndexCatalog{objects:LfsIndexObject[];totalIndexEntries:number;pointerPaths:number;skippedNonUtf8:number}
export interface LfsScanProgress{scanned:number;total:number;objects:number}
/** Read only. A cancelled/failed scan never produces a complete inventory. */
export async function collectLfsIndex(read:(offset:number)=>Promise<LfsIndexPage>,cancelled:()=>boolean,onProgress:(progress:LfsScanProgress)=>void):Promise<LfsIndexCatalog>{
 const objects=new Map<string,LfsIndexObject>(),paths=new Set<string>();
 let offset=0,total:number|undefined,pointerPaths=0,skippedNonUtf8=0,storedBytes=0;
 const encoder=new TextEncoder();
 for(;;){
  if(cancelled())throw new Error('LFS_INDEX_SCAN_CANCELLED');
  const page=await read(offset);
  if(cancelled())throw new Error('LFS_INDEX_SCAN_CANCELLED');
  if(page.offset!==offset||!Number.isSafeInteger(page.totalIndexEntries)||page.totalIndexEntries<offset||(total!==undefined&&page.totalIndexEntries!==total)||!Number.isSafeInteger(page.skippedNonUtf8)||page.skippedNonUtf8<0)throw new Error('LFS_INDEX_SCAN_CHANGED: reload the repository before scanning again.');
  total=page.totalIndexEntries;
  const end=Math.min(offset+1000,total);
  if(page.nextOffset!==(end<total?end:null)||page.entries.length>end-offset)throw new Error('LFS_INDEX_PAGE_INVALID');
  for(const entry of page.entries){
   if(paths.has(entry.path))throw new Error('LFS_INDEX_DUPLICATE_PATH');
   const pointer=inspectLfsPointer(entry.text);if(!pointer)throw new Error('LFS_INDEX_POINTER_INVALID');
   storedBytes+=encoder.encode(entry.path).length+encoder.encode(entry.text).length;
   if(++pointerPaths>20000||storedBytes>16*1024*1024)throw new Error('LFS_INDEX_CATALOG_LIMIT: use the paged view for this repository. No complete inventory was produced.');
   paths.add(entry.path);
   const existing=objects.get(pointer.oid);
   if(existing){if(existing.size!==pointer.size)throw new Error('LFS_INDEX_CONFLICTING_OBJECT_SIZE');existing.paths.push(entry.path);}
   else objects.set(pointer.oid,{oid:pointer.oid,size:pointer.size,path:entry.path,text:entry.text,paths:[entry.path]});
  }
  skippedNonUtf8+=page.skippedNonUtf8;
  onProgress({scanned:end,total,objects:objects.size});
  if(page.nextOffset===null)return {objects:[...objects.values()],totalIndexEntries:total,pointerPaths,skippedNonUtf8};
  offset=page.nextOffset;
 }
}
