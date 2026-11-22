import type { ChangeEvent } from 'react';
export function AmendControl({checked,writable,hasHead,onChange}:{checked:boolean;writable:boolean;hasHead:boolean;onChange:(event:ChangeEvent<HTMLInputElement>)=>void}) {
 return <><label className="amend"><input type="checkbox" checked={checked} disabled={!writable||!hasHead} onChange={onChange}/>Amend previous commit</label>{checked&&<p role="status">Replaces current HEAD with staged content. Original author and parents are preserved; identity below is the committer.</p>}</>;
}
