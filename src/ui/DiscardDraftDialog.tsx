export function DiscardDraftDialog({onCancel,onDiscard}:{onCancel:()=>void;onDiscard:()=>void}){
 return <div className="modal-backdrop discard-provider-draft"><section className="modal" role="dialog" aria-modal="true" aria-label="Discard unsent changes"><h2>Discard unsent changes?</h2><p>This will discard the description, review, comment or metadata drafts in this panel.</p><div className="setting-row"><button autoFocus data-modal-close onClick={onCancel}>Keep editing</button><button onClick={onDiscard}>Discard and continue</button></div></section></div>;
}
