import {RepositoryScopePicker} from './RepositoryScopePicker';
import {useLaunchpadRepositoryScope} from '../launchpadRepositoryScope';
import {useId,type ReactNode} from 'react';
export function ProviderFeedHeader({title,scope,inputLabel,loadLabel,createLabel,disabled,disabledReason,onScope,onLoad,onCreate,children}:{title:string;scope:string;inputLabel:string;loadLabel:string;createLabel:string;disabled:boolean;disabledReason?:string;onScope:(scope:string)=>void;onLoad:()=>void;onCreate:()=>void;children:ReactNode}){
 const helpId=useId();
 const {suggestions}=useLaunchpadRepositoryScope();
 const guidance=disabled?(disabledReason??'Actions are temporarily unavailable while workspace settings load or another operation finishes.'):!scope.trim()?`Enter a public GitHub repository as owner/repository, then select ${loadLabel}.`:'';
 return <header className="provider-feed-header"><form onSubmit={event=>{event.preventDefault();if(!disabled&&scope.trim())onLoad();}}><RepositoryScopePicker suggestions={suggestions} value={scope} label={`${title} repository from current remotes`} disabled={disabled} onSelect={onScope}/><input aria-describedby={helpId} aria-label={inputLabel} placeholder="owner/repository" value={scope} maxLength={201} disabled={disabled} onChange={event=>onScope(event.target.value)}/><button title={guidance||undefined} disabled={disabled||!scope.trim()} type="submit">{loadLabel}</button><span className="grow"/><button title={guidance||undefined} type="button" disabled={disabled||!scope.trim()} onClick={onCreate}>{createLabel}</button><details><summary aria-label={`${title} information`}>ⓘ</summary><div className="provider-feed-help">{children}</div></details></form><p id={helpId} className="history-base" role="status">{guidance}</p></header>;
}
