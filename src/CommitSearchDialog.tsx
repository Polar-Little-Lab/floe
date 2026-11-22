import {useEffect,useMemo,useState} from 'react';
import {referenceMatcher} from './referenceFilter';
import type {CommitSearchResults} from './api';
import {X} from './OriginalIcons';
export function CommitSearchDialog({results,busy,error,onOpen,onClose,onSearch}:{onSearch?:(query:string)=>void;results:CommitSearchResults;busy:boolean;error:string;onOpen:(hit:CommitSearchResults['hits'][number])=>void;onClose:()=>void}) {
 const [search,setSearch]=useState(results.query);const [filter,setFilter]=useState('');const [author,setAuthor]=useState('');const [sort,setSort]=useState('history');
 useEffect(()=>{setSearch(results.query);setFilter('');setAuthor('');},[results]);
 const authors=useMemo(()=>[...new Set(results.hits.map(hit=>hit.commit.author))].sort((a,b)=>a.localeCompare(b)),[results]);
 const hits=useMemo(()=>{const matches=referenceMatcher(filter);const list=results.hits.filter(hit=>(!author||hit.commit.author===author)&&matches(`${hit.commit.oid} ${hit.commit.author} ${hit.commit.summary}`));if(sort!=='history')list.sort((a,b)=>sort==='newest'?b.commit.time-a.commit.time:a.commit.time-b.commit.time);return list;},[results,filter,author,sort]);
 const validSearch=!!search.trim()&&new TextEncoder().encode(search.trim()).length<=512;
 return <div className="modal-backdrop"><section className="modal commit-search-results" role="dialog" aria-modal="true" aria-label="Commit search results">
  <div className="modal-header"><h2>Search history</h2><button disabled={busy} data-modal-close aria-label="Close search results" onClick={onClose}><X size={18}/></button></div>
  {onSearch&&<form className="commit-search-query" onSubmit={event=>{event.preventDefault();if(!busy&&validSearch)onSearch(search.trim());}}><input aria-label="Search repository history" value={search} maxLength={512} onChange={event=>setSearch(event.target.value)} disabled={busy}/><button disabled={busy||!validSearch}>Search history</button>{search.trim()&&!validSearch&&<small role="alert">Query exceeds 512 UTF-8 bytes.</small>}</form>}
  <p><strong>{results.query}</strong> · {results.hits.length} results · {results.scanned.toLocaleString()} commits searched</p>
  <p className="history-base">Searches local branches, remote-tracking branches and tags. Up to 20,000 commits, 200 results or 3 seconds per search; the first 65,536 message characters are searched.</p>
  {results.truncated&&<p role="status">Search limit reached. Refine your query; this is a partial result list.</p>}
  <div className="commit-search-refine"><input aria-label="Filter returned commits" placeholder="Filter these results: title, author, SHA" value={filter} onChange={event=>setFilter(event.target.value)}/><select aria-label="Filter returned commits by author" value={author} onChange={event=>setAuthor(event.target.value)}><option value="">All authors</option>{authors.map(value=><option key={value} value={value}>{value}</option>)}</select><select aria-label="Sort returned commits" value={sort} onChange={event=>setSort(event.target.value)}><option value="history">History order</option><option value="newest">Newest first</option><option value="oldest">Oldest first</option></select>{(filter||author)&&<button onClick={()=>{setFilter('');setAuthor('');}}>Clear filters</button>}</div>
  <p className="history-base">Showing {hits.length}/{results.hits.length} returned commits. These filters do not search additional history.</p>
  {!hits.length&&<p>{results.hits.length?'No returned commits match these filters.':'No matching commits in the searched history.'}</p>}
  {hits.map(hit=><button className="commit-search-hit" disabled={busy} key={hit.commit.oid} onClick={()=>onOpen(hit)}><strong>{hit.commit.summary}</strong><span>{hit.commit.oid.slice(0,10)} · {hit.commit.author} · {new Date(hit.commit.time*1000).toLocaleDateString()}</span></button>)}
  {error&&<p role="alert">{error}</p>}
 </section></div>;
}
