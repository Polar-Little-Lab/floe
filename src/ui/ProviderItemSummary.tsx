import {GitBranch,Tag} from '../OriginalIcons';
export function ProviderItemSummary({number,title,labels,milestone,disabled,onOpen}:{number:number;title:string;labels?:string[]|null;milestone?:{title:string}|null;disabled:boolean;onOpen:()=>void}){
 return <div className="provider-item-summary"><button className="provider-item-title" disabled={disabled} onClick={onOpen}>{title} <span className="provider-item-number">#{number}</span></button>{!!labels?.length&&<div className="provider-item-labels">{labels.map(label=><span key={label} title={label}><Tag size={10}/>{label}</span>)}</div>}{milestone&&<small className="provider-item-milestone" title={`Milestone: ${milestone.title}`}>{milestone.title}</small>}</div>;
}
export function ProviderRepositorySummary({repository,head,base}:{repository:string;head?:string;base?:string}){
 return <div className="provider-repository-summary"><span>{repository}</span>{head&&<span className="provider-branch-badge" title={base?`${head} → ${base}`:head}><GitBranch size={12}/>{head}</span>}{base&&<small>→ {base}</small>}</div>;
}
export function ProviderPeople({names,empty='None listed'}:{names:string[]|null;empty?:string}){
 if(names===null)return <span className="provider-people-empty">Unknown</span>;
 if(!names.length)return <span className="provider-people-empty">{empty}</span>;
 return <div className="provider-people">{names.map(name=><span key={name} className="provider-person" title={name}><span className="provider-person-initial" aria-hidden="true">{Array.from(name).slice(0,2).join('').toUpperCase()}</span><span>{name}</span></span>)}</div>;
}
