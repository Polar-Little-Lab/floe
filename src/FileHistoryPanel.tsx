import {useState} from 'react';
import { ArrowLeft } from './OriginalIcons';
import {CopyTextButton} from './CopyTextButton';
import {referenceMatcher} from './referenceFilter';
import type { FileHistory } from './types';
export function FileHistoryPanel({page,busy,selected,onPage,onSelect,onClose,onCommit}:{page:FileHistory;busy:boolean;selected?:string;onPage:(offset:number)=>void;onSelect:(oid:string)=>void;onClose:()=>void;onCommit?:(oid:string)=>void}) {
 const [query,setQuery]=useState('');
 const [author,setAuthor]=useState('');
 const [status,setStatus]=useState('');
 const matches=referenceMatcher(query);
 const authors=[...new Set(page.entries.map(entry=>entry.author))].sort();
 const statuses=[...new Set(page.entries.map(entry=>entry.status))].sort();
 const entries=page.entries.filter(entry=>(!author||entry.author===author)&&(!status||entry.status===status)&&matches(`${entry.summary} ${entry.author} ${entry.oid} ${entry.path} ${entry.previousPath??''} ${entry.status}`));
 const selectedIndex=entries.findIndex(entry=>entry.oid===selected);
 const selectedEntry=page.entries.find(entry=>entry.oid===selected);
 const resetFilters=()=>{setQuery('');setAuthor('');setStatus('');};
 const move=(step:number)=>{const entry=entries[selectedIndex<0?0:selectedIndex+step];if(!busy&&entry)onSelect(entry.oid);};
 return <><div className="details-header"><button disabled={busy} aria-label="Close file history" data-modal-close onClick={onClose}><ArrowLeft size={18}/></button><span>File history</span></div>
 <div className="commit-details file-history-details">
  <h2>{page.path}</h2><CopyTextButton text={page.path} label="Copy file path"/>
  <p className="history-base">First-parent history · follows exact-content renames</p>
  <p>Commits scanned: {page.scanned ? `${page.offset + 1}–${page.offset + page.scanned}` : '0'}</p>
  <input className="wide-input" aria-label="Filter current file history page" placeholder="Filter this history page…" maxLength={200} value={query} onChange={event=>setQuery(event.target.value)}/>
  <div className="file-history-filters">
   <select aria-label="File history author" value={author} onChange={event=>setAuthor(event.target.value)}><option value="">All authors</option>{author&&!authors.includes(author)&&<option value={author}>{author} (not on this page)</option>}{authors.map(value=><option key={value} value={value}>{value}</option>)}</select>
   <select aria-label="File history change type" value={status} onChange={event=>setStatus(event.target.value)}><option value="">All change types</option>{status&&!statuses.includes(status)&&<option value={status}>{status} (not on this page)</option>}{statuses.map(value=><option key={value} value={value}>{value}</option>)}</select>
   {(query||author||status)&&<button onClick={resetFilters}>Clear filters</button>}
  </div>
  <p>{entries.length} of {page.entries.length} changes on this page. Filters do not search other pages.</p>
  <nav className="file-history-navigation" aria-label="File history changes">
   <button disabled={busy||selectedIndex<=0} onClick={()=>move(-1)}>Previous change</button>
   <span>{selectedIndex<0?'No visible selection':`${selectedIndex+1} / ${entries.length}`}</span>
   <button disabled={busy||!entries.length||selectedIndex===entries.length-1} onClick={()=>move(1)}>{selectedIndex<0?'First change':'Next change'}</button>
  </nav>
  {selected&&selectedIndex<0&&<p role="status">The displayed change {selected.slice(0,7)} is not in this filtered list.</p>}
  {selectedEntry&&<div className="file-history-selection-tools"><CopyTextButton text={selectedEntry.oid} label="Copy selected commit SHA"/><CopyTextButton text={selectedEntry.path} label="Copy historical path"/>{selectedEntry.previousPath&&<CopyTextButton text={selectedEntry.previousPath} label="Copy previous path"/>}</div>}
  {entries.map(entry=><div className="file-history-entry" key={entry.oid}><button disabled={busy} aria-current={selected===entry.oid?'true':undefined} className={`history-file ${selected===entry.oid?'chosen':''}`} onClick={()=>onSelect(entry.oid)}><span><strong>{entry.summary}</strong><small>{entry.oid.slice(0,7)} · {entry.author} · {new Date(entry.time*1000).toLocaleDateString()}</small><small>{entry.status} · {entry.previousPath?`${entry.previousPath} → ${entry.path}`:entry.path}</small></span></button>{onCommit&&<button className="history-open-commit" disabled={busy} onClick={()=>onCommit(entry.oid)} aria-label={`Open full commit ${entry.oid}`}>Open full commit</button>}</div>)}
  {!!page.entries.length&&!entries.length&&<p>No matches on this history page.</p>}{!page.entries.length&&<p>No file changes in this page.</p>}
  <div className="setting-row"><button disabled={busy||page.offset===0} onClick={()=>onPage(Math.max(0,page.offset-100))}>Previous</button><button disabled={busy||page.nextOffset===null} onClick={()=>page.nextOffset!==null&&onPage(page.nextOffset)}>Next</button></div>{page.limitReached&&<p>History scan limit reached (10,000 commits).</p>}
 </div></>;
}
