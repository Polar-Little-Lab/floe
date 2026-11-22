import type {Snapshot} from './types';
import {X} from './OriginalIcons';
export function DiscardChangesDialog({snapshot,busy,error,onClose,onDiscard}:{snapshot:Snapshot;busy:boolean;error:string;onClose:()=>void;onDiscard:()=>void}) {
 return <div className="modal-backdrop"><section className="modal" role="dialog" aria-modal="true" aria-label="Discard all changes">
  <div className="modal-header"><h2>Discard all changes?</h2><button disabled={busy} data-modal-close aria-label="Close discard changes" onClick={onClose}><X size={18}/></button></div>
  <p>Restore staged and unstaged files to the current commit and remove untracked files from the working folder. Ignored files are left unchanged.</p>
  <p>A recovery stash named <strong>Discard backup</strong> is saved first. You can restore it from <strong>STASHES</strong> with Apply or Pop and “Restore index”.</p>
  <p>{snapshot.name} · {snapshot.changes.length} changed files</p>
  <ul className="discard-file-list">{snapshot.changes.map(change=><li key={change.id}>{change.path}</li>)}</ul>
  {error&&<p role="alert" className="readonly-note">{error}</p>}
  <div className="setting-row"><button autoFocus disabled={busy} data-modal-close onClick={onClose}>Keep changes</button><button disabled={busy} onClick={onDiscard}>{busy?'Saving backup…':'Back up and discard'}</button></div>
 </section></div>;
}
