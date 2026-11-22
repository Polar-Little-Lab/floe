import {findPreferences,type PreferenceSearchResult} from './preferenceSearch';
import {useEffect,useRef,useState,type ReactNode} from 'react';
import {ArrowLeft,Settings,Users,Pencil,Columns3,Keyboard,Bell,Check,Cloud,Sparkles,FileText,Terminal,FlaskConical,ListFilter} from './OriginalIcons';
import {ScreenNavigation,WorkspaceScreen} from './ui/WorkspaceScreen';
export type PreferencesPage='general'|'profiles'|'ui'|'editor'|'shortcuts'|'notifications'|'commit'|'external'|'gitflow'|'integrations';
type Page=PreferencesPage;
function PreferencePage({id,title,active,children}:{id:PreferencesPage;title:string;active:boolean;children:ReactNode}){
 const [visited,setVisited]=useState(active);
 useEffect(()=>{if(active)setVisited(true);},[active]);
 return <div data-preference-page={id} hidden={!active}><h2>{title}</h2>{(active||visited)&&children}</div>;
}

const pages=[{id:'general',label:'General',icon:<Settings size={19}/>},{id:'profiles',label:'Profiles',icon:<Users size={19}/>},{id:'external',label:'External Tools',icon:<Columns3 size={19}/>},{id:'notifications',label:'Notifications',icon:<Bell size={19}/>},{id:'ui',label:'UI Customization',icon:<Pencil size={19}/>},{id:'editor',label:'Editor',icon:<Columns3 size={19}/>},{id:'shortcuts',label:'Keyboard Shortcuts',icon:<Keyboard size={19}/>} ] as const;
export function PreferencesScreen({initialPage='general',onClose,general,profiles,appearance,editor,shortcuts,notifications,external,commit,gitflow,integrations,repositoryName}:{initialPage?:PreferencesPage;onClose:()=>void;general:ReactNode;profiles:ReactNode;appearance:ReactNode;editor:ReactNode;shortcuts:ReactNode;notifications?:ReactNode;external?:ReactNode;commit?:ReactNode;gitflow?:ReactNode;integrations?:ReactNode;repositoryName?:string}){
 const [page,setPage]=useState<Page>(initialPage);
 const [query,setQuery]=useState('');
 const [searchOpen,setSearchOpen]=useState(false);
 const filterRef=useRef<HTMLButtonElement>(null);
 const contentRef=useRef<HTMLDivElement>(null);
 const matches=findPreferences(query,!!commit);
 const selectResult=(result:PreferenceSearchResult)=>{
  setPage(result.page);setQuery('');
  requestAnimationFrame(()=>{
   const panel=contentRef.current?.querySelector<HTMLElement>(`[data-preference-page="${result.page}"]`);
   const control=Array.from(panel?.querySelectorAll<HTMLElement>('[aria-label]')??[]).find(node=>node.getAttribute('aria-label')===result.label);
   const field=control?.closest<HTMLElement>('.settings-field')??control??Array.from(panel?.querySelectorAll<HTMLElement>('.settings-field')??[]).find(node=>node.querySelector('.settings-field-label')?.textContent===result.label);
   field?.scrollIntoView({block:'center'});
   const target=control&&!control.matches(':disabled')?control:field?.querySelector<HTMLElement>('input:not(:disabled),select:not(:disabled),button:not(:disabled),textarea:not(:disabled)');
   target?.focus({preventScroll:true});
  });
 };
 const repositoryPages=[...(gitflow?[{id:'gitflow' as const,label:'Gitflow',icon:<Columns3 size={19}/>}]:[]),{id:'commit' as const,label:'Commit',icon:<Pencil size={19}/>}];
 const visibleRepositoryPages=repositoryPages.filter(item=>item.label.toLowerCase().includes(query.toLowerCase()));
 // Preserve original category positions even where the settings surface is not implemented.
 const navigationItems:{id:string;label:string;icon:ReactNode;disabled?:boolean;unavailableReason?:string}[]=[
  pages[0],pages[1],
  {id:'ssh',label:'SSH',icon:<Check size={19}/>,disabled:true,unavailableReason:'SSH preferences are not implemented.'},
  {id:'integrations',label:'Integrations',icon:<Cloud size={19}/>,disabled:!integrations,unavailableReason:integrations?undefined:'Provider connection preferences are not available.'},
  {id:'ai',label:'GitKraken AI',icon:<Sparkles size={19}/>,disabled:true,unavailableReason:'AI features are excluded from FLOE.'},
  pages[2],pages[3],pages[4],
  {id:'signing',label:'Commit Signing',icon:<FileText size={19}/>,disabled:true,unavailableReason:'Commit signing preferences are not implemented.'},
  pages[5],
  {id:'terminal',label:'In-App Terminal',icon:<Terminal size={19}/>,disabled:true,unavailableReason:'In-app terminal preferences are not implemented.'},
  {id:'experimental',label:'Experimental',icon:<FlaskConical size={19}/>,disabled:true,unavailableReason:'Experimental preferences are not implemented.'},
  pages[6],
 ];
 const visiblePages=navigationItems.filter(item=>item.label.toLowerCase().includes(query.toLowerCase()));
 const contents:Record<Page,ReactNode>={general,profiles,ui:appearance,editor,shortcuts,notifications,external,commit,gitflow,integrations};
 return <WorkspaceScreen label="설정" className="preferences-screen" onClose={onClose} sidebar={<><button className="preferences-exit" autoFocus data-modal-close aria-label="설정 닫기" onClick={onClose}><ArrowLeft size={20}/>Exit Preferences</button><p className="profile-caption">Current profile</p><button className="preferences-profile" onClick={()=>{setPage('profiles');setQuery('');}}><Users size={22}/>Default Profile</button><p className="profile-caption">Preferences</p><ScreenNavigation label="Preferences categories" vertical items={visiblePages} value={page} onChange={next=>{if(pages.some(item=>item.id===next)||(next==='integrations'&&integrations))setPage(next as Page);}}/>{commit&&<><p className="profile-caption">Repo-Specific Preferences</p><p className="profile-caption">Repo: {repositoryName??'No repository'}</p><ScreenNavigation label="Repository preferences categories" vertical items={visibleRepositoryPages} value={page} onChange={setPage}/></>}{!matches.length&&!visiblePages.length&&(!commit||!visibleRepositoryPages.length)&&<p className="profile-caption">No matching preferences.</p>}</>}>
  <div className="preferences-filter-bar"><button ref={filterRef} aria-label="Filter preferences" aria-expanded={searchOpen} aria-controls="preferences-filter-panel" onClick={()=>{setSearchOpen(!searchOpen);if(searchOpen)setQuery('');}}><ListFilter size={20}/></button></div>
  {searchOpen&&<div id="preferences-filter-panel" className="preferences-filter-panel" onKeyDown={event=>{if(event.key==='Escape'){event.preventDefault();event.stopPropagation();setSearchOpen(false);setQuery('');filterRef.current?.focus();}}}><input autoFocus className="preferences-search" aria-label="Search preferences" placeholder="Search preferences" value={query} onChange={e=>setQuery(e.target.value)} onKeyDown={event=>{if(event.key==='Enter'&&matches[0]){event.preventDefault();selectResult(matches[0]);}}}/>{query&&<button className="preferences-search-clear" onClick={()=>setQuery('')}>Clear search</button>}{matches.length>0&&<nav className="preference-search-results" aria-label="Matching settings">{matches.map(item=><button key={item.label} onClick={()=>selectResult(item)}><span>{item.label}</span><small>{item.page==='ui'?'UI Customization':item.page}</small></button>)}</nav>}</div>}
  <div ref={contentRef}>{[...pages,...(integrations?[{id:'integrations' as const,label:'Integrations'}]:[]),...(commit?repositoryPages:[])].map(item=><PreferencePage key={item.id} id={item.id} title={item.label} active={page===item.id}>{contents[item.id]}</PreferencePage>)}</div>
 </WorkspaceScreen>;
}
