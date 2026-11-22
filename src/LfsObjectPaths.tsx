import {useEffect,useMemo,useState} from 'react';
/** Bounded path rendering for objects referenced by many indexed files. */
export function LfsObjectPaths({paths,query}:{paths:readonly string[];query:string}){
 const [offset,setOffset]=useState(0);
 const matches=useMemo(()=>{const term=query.toLocaleLowerCase();return term?paths.filter(path=>path.toLocaleLowerCase().includes(term)):paths;},[paths,query]);
 useEffect(()=>setOffset(0),[paths,query]);
 const start=offset<matches.length?offset:0;
 return <details><summary>{query?`${matches.length} matching paths`:`Show ${paths.length} paths`}</summary>
  {matches.slice(start,start+100).map(path=><p key={path}>{path}</p>)}
  {!matches.length&&<p>No matching paths.</p>}
  {matches.length>100&&<div className="setting-row"><button type="button" disabled={start===0} onClick={()=>setOffset(Math.max(0,start-100))}>Previous paths</button><span>{start+1}–{Math.min(start+100,matches.length)} / {matches.length}</span><button type="button" disabled={start+100>=matches.length} onClick={()=>setOffset(start+100)}>Next paths</button></div>}
 </details>;
}
