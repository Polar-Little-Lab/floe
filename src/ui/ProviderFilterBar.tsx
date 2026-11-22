import {createPortal} from 'react-dom';
import {useLaunchpadSlots,type LaunchpadFilterScope} from './LaunchpadSlots';
import {useLaunchpadSearch} from './LaunchpadSearch';
import type {ReactNode} from 'react';
import {Search,X} from '../OriginalIcons';
export function ProviderFilterBar({label,query,onQuery,onRefresh,onReset,active,disabled,loaded,summary,children,scope,canRefresh=loaded,onStop,loading=false}:{label:string;query:string;onQuery:(value:string)=>void;onRefresh:()=>void;onReset:()=>void;active:boolean;disabled:boolean;loaded:boolean;summary?:string;children:ReactNode;scope?:LaunchpadFilterScope;canRefresh?:boolean;onStop?:()=>void;loading?:boolean}){
 const sharedSearch=useLaunchpadSearch(),slots=useLaunchpadSlots(scope);
 const refresh=<><button aria-label={`Refresh ${label.toLowerCase()}`} title={loaded?'Refresh loaded tasks':'Load tasks'} disabled={disabled||!canRefresh} onClick={onRefresh}>↻</button>{loading&&onStop&&<button onClick={onStop}>Stop loading</button>}</>;
 const filters=<fieldset className="provider-filter-options" disabled={disabled||!loaded} aria-label={`${label} metadata filters`}>{children}{active&&<button onClick={onReset}>Clear filters</button>}</fieldset>;
 return <section className="provider-filter-bar" aria-label={`${label} filters`}>
  {slots?.refresh?createPortal(refresh,slots.refresh):<div className="provider-search-row">{refresh}{!sharedSearch&&<label className="provider-search"><Search size={16}/><input aria-label={`Filter loaded ${label.toLowerCase()}`} placeholder="Search" value={query} maxLength={4096} disabled={!loaded} onChange={event=>onQuery(event.target.value)}/>{query&&<button aria-label="Clear search" onClick={()=>onQuery('')}><X size={14}/></button>}</label>}</div>}
  {slots?.filters?createPortal(filters,slots.filters):filters}
  {summary&&<p className="history-base" role="status">{summary}</p>}
 </section>;
}
