import {useState} from 'react';
import type {CheckpointListing} from './api';
import {X} from './OriginalIcons';
export function RecoveryCheckpointsDialog({listing,busy,writable,error,onClose,onCreate}:{listing:CheckpointListing;busy:boolean;writable:boolean;error:string;onClose:()=>void;onCreate:(entry:CheckpointListing['entries'][number],name:string)=>void}) {
 const [query,setQuery]=useState('');
 const [selected,setSelected]=useState('');
 const [name,setName]=useState('');
 const entry=listing.entries.find(item=>item.reference===selected);
 const visible=listing.entries.filter(item=>`${item.summary} ${item.oid} ${item.author}`.toLowerCase().includes(query.toLowerCase()));
 return <div className="modal-backdrop"><form className="modal recovery-checkpoints" role="dialog" aria-modal="true" aria-label="Recovery checkpoints" onSubmit={event=>{event.preventDefault();if(entry&&name.trim()&&!busy&&writable)onCreate(entry,name.trim());}}>
  <div className="modal-header"><h2>Recovery checkpoints</h2><button type="button" disabled={busy} data-modal-close aria-label="Close recovery checkpoints" onClick={onClose}><X size={18}/></button></div>
  <p>Local commits retained by FLOE operations. Create a separate recovery branch to inspect one without changing your current branch, staged changes or files. Stash checkpoints may contain combined working changes.</p>
  <input autoFocus className="wide-input" aria-label="Filter recovery checkpoints" placeholder="Filter message, author or SHA" value={query} onChange={e=>{setQuery(e.target.value);setSelected('');}} disabled={busy}/>
  {listing.truncated&&<p role="status">The checkpoint list is limited. Up to 500 entries from 5,000 scanned references are shown.</p>}
  <select size={7} className="checkpoint-list" aria-label="Choose recovery checkpoint" value={selected} disabled={busy} onChange={e=>{setSelected(e.target.value);const item=listing.entries.find(c=>c.reference===e.target.value);if(item)setName(`recovery/${item.oid.slice(0,7)}`);}}>{visible.map(item=><option key={item.reference} value={item.reference}>{item.oid.slice(0,8)} · {item.summary}</option>)}</select>
  {!visible.length&&<p>No matching checkpoints.</p>}
  {entry&&<><p><code>{entry.oid}</code><br/>{entry.author} · Commit date: {new Date(entry.time*1000).toLocaleString()}</p><p className="history-base">{entry.reference}</p></>}
  <input className="wide-input" aria-label="Recovery branch name" placeholder="recovery/my-work" value={name} maxLength={200} disabled={busy} onChange={e=>setName(e.target.value)}/>
  {error&&<p role="alert">{error}</p>}
  <div className="setting-row"><button type="button" data-modal-close disabled={busy} onClick={onClose}>Close</button><button disabled={!entry||!name.trim()||busy||!writable}>Create recovery branch</button></div>
 </form></div>;
}
