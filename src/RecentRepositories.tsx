export function RecentRepositories({paths,busy,onOpen,onForget}:{paths:string[];busy:boolean;onOpen:(path:string)=>void;onForget:(path:string)=>void}) {
 if(!paths.length)return null;
 return <section aria-label="Recent repositories" className="recent-repositories"><h3>Recent repositories</h3>{paths.map(path=><div className="recent-repository" key={path}><button type="button" disabled={busy} title={path} onClick={()=>onOpen(path)}>{path}</button><button type="button" disabled={busy} aria-label={`Remove from recent: ${path}`} title="Remove from this list only" onClick={()=>onForget(path)}>Remove</button></div>)}</section>;
}
