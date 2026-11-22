import {createContext,useContext,useState,type ReactNode} from 'react';
import {Search,X} from '../OriginalIcons';
import {TaskGroupControls} from './TaskGroup';
type SearchState={query:string;setQuery:(value:string)=>void};
const Context=createContext<SearchState|null>(null);
export function LaunchpadSearchProvider({children}:{children:ReactNode}){
 const [query,setQuery]=useState('');
 return <Context.Provider value={{query,setQuery}}>{children}</Context.Provider>;
}
export function useLaunchpadSearch(){return useContext(Context);}
/** Standalone feeds retain a local search; Launchpad feeds share the toolbar query. */
export function useTaskSearch():[string,(value:string)=>void]{
 const shared=useLaunchpadSearch();const [query,setQuery]=useState('');
 return shared?[shared.query,shared.setQuery]:[query,setQuery];
}
export function LaunchpadTaskToolbar({onRefreshHost}:{onRefreshHost?:(element:HTMLSpanElement|null)=>void}){
 const search=useLaunchpadSearch();
 if(!search)return null;
 return <div className="launchpad-task-toolbar"><TaskGroupControls/><span className="launchpad-refresh-slot" ref={onRefreshHost}/><label className="provider-search"><Search size={16}/><input aria-label="Search loaded Launchpad tasks" placeholder="Search" title="Filter loaded pull requests, issues and WIPs" maxLength={4096} value={search.query} onChange={event=>search.setQuery(event.target.value)}/>{search.query&&<button aria-label="Clear Launchpad search" onClick={()=>search.setQuery('')}><X size={14}/></button>}</label></div>;
}
