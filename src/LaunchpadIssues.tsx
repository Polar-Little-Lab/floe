import {ProviderItemSummary,ProviderRepositorySummary,ProviderPeople} from './ui/ProviderItemSummary';
import {MetadataFilter} from './ui/MetadataFilter';
import {Tag,Users,FileText} from './OriginalIcons';
import {useTaskSearch} from './ui/LaunchpadSearch';
import {useProviderPanel} from './useProviderPanel';
import {useLaunchpadRepositoryScope} from './launchpadRepositoryScope';
import {TaskGroup} from './ui/TaskGroup';
import {ProviderFilterBar} from './ui/ProviderFilterBar';
import {ProviderFeedHeader} from './ui/ProviderFeedHeader';
import {ProviderTable} from './ui/ProviderTable';
import {ProviderItemPanel} from './ProviderItemPanel';
import {TaskPreferenceControls,useTaskPreferences,visiblePreferenceItems,type TaskPreferences} from './TaskPreferenceControls';
import {taskIdentity,taskIdentityKey} from './taskSnoozes';
import {useEffect,useRef,useState} from 'react';
import {api} from './api';
import {CopyTextButton} from './CopyTextButton';
import {IssueFeed,filterIssues,type IssueState} from './providerIssues';
export function LaunchpadIssues({disabled=false,preferences}:{disabled?:boolean;preferences?:TaskPreferences}){
 const {scope,setScope}=useLaunchpadRepositoryScope();
 const [query,setQuery]=useTaskSearch();
 const {panel,setPanel,requestPanel,registerNavigationGuard}=useProviderPanel<{repository:string;number?:number}>(previous=>{void feed.current?.load(previous.repository);});
 const [assignee,setAssignee]=useState<string|null>(null),[label,setLabel]=useState<string|null>(null),[milestone,setMilestone]=useState<string|null>(null);
 const [state,setState]=useState<IssueState>({repository:'',items:[],nextPage:null,loading:false,error:'',loaded:false});
 const feed=useRef<IssueFeed|null>(null);
 if(!feed.current)feed.current=new IssueFeed(api.readGithubIssues,setState);
 useEffect(()=>()=>feed.current?.dispose(),[]);
 useEffect(()=>{setAssignee(null);setLabel(null);setMilestone(null);},[state.repository]);
 const preferenceState=useTaskPreferences(preferences);
 const visible=visiblePreferenceItems(filterIssues(state.items,query,{assignee,label,milestone}),state.repository,'issues',preferenceState);
 const labels=[...new Set(state.items.flatMap(item=>item.labels??[]))].sort();
 const assignees=[...new Map(state.items.flatMap(item=>item.assignees??[]).map(value=>[value.toLocaleLowerCase(),value])).values()].sort();
 const milestones=[...new Map(state.items.flatMap(item=>item.milestone?[[item.milestone.number,item.milestone] as const]:[])).values()];
 const select=(name:string,value:string|null,set:(v:string|null)=>void,options:{value:string;title:string}[])=>
  <MetadataFilter label={`Filter issue ${name}`} allLabel={`All ${name}s`} value={value} options={options} onChange={set} disabled={!state.loaded||disabled||state.loading} icon={name==='label'?<Tag size={15}/>:name==='assignee'?<Users size={15}/>:<FileText size={15}/>}/>;

 return <section aria-label="GitHub issues">
  <ProviderFeedHeader title="Issues" scope={scope} inputLabel="GitHub issue repository" loadLabel="Load issues" createLabel="Create Issue" disabled={disabled||state.loading} disabledReason={state.loading?'Loading issues…':undefined} onScope={setScope} onLoad={()=>void feed.current?.load(scope)} onCreate={()=>requestPanel({repository:scope.trim()})}><p>Public GitHub repositories. Filters apply to loaded results. Pins and snoozes are saved on this device.</p></ProviderFeedHeader>
  {panel&&<ProviderItemPanel registerNavigationGuard={registerNavigationGuard} repository={panel.repository} number={panel.number} pull={false} onClose={refresh=>{const repository=panel.repository;setPanel(null);if(refresh)void feed.current?.load(repository);}}/>}
  {state.loading&&<p role="status">Loading issues…</p>}
  {preferenceState.error&&<p role="alert">{preferenceState.error} Reload workspaces before retrying.</p>}
  {state.error&&<p role="alert">{state.error}</p>}
  <ProviderFilterBar scope="issues" label="Issues" query={query} onQuery={setQuery} onRefresh={()=>void feed.current?.load(state.repository)} onReset={()=>{setQuery('');setAssignee(null);setLabel(null);setMilestone(null);}} active={!!query||assignee!==null||label!==null||milestone!==null} disabled={disabled||state.loading} loaded={state.loaded} summary={state.loaded?`${state.repository} · ${visible.length} of ${state.items.length} loaded${state.nextPage!==null?' · more available':''}`:undefined}>
    {select('assignee',assignee,setAssignee,assignees.map(value=>({value,title:value})))}
    {select('label',label,setLabel,labels.map(value=>({value,title:value})))}
    {select('milestone',milestone,setMilestone,milestones.map(m=>({value:m.number,title:`${m.title} (#${m.number})`})))}
  </ProviderFilterBar>
  {state.loaded&&!state.loading&&!state.items.length&&<p>{state.nextPage!==null?'No issues in loaded pages. More pages are available.':'No open issues.'}</p>}
  {state.loaded&&state.items.length>0&&!visible.length&&<p>No matching loaded issues.</p>}
  <TaskGroup id={`single:issues:${state.repository}`} title={state.repository?`${state.repository} · Issues`:'Issues'} count={visible.length} warning={!!state.error}>
  {!visible.length&&<ProviderTable label="Issues" rows={[]} empty={<><h3>{state.loaded?'No matching issues':'Your issues appear here'}</h3><p>{state.loaded?'Change the filters or choose another repository.':'Select a workspace or enter owner/repository above to load issues.'}</p></>}/> }
  {visible.length>0&&<ProviderTable label="Issues" rows={visible.map(item=>({id:item.id,updatedAt:item.updatedAt,selected:panel?.number===item.number&&panel.repository.toLowerCase()===state.repository.toLowerCase(),onOpen:disabled||state.loading?undefined:()=>requestPanel({repository:state.repository,number:item.number}),status:'Open',item:<ProviderItemSummary number={item.number} title={item.title} labels={item.labels} milestone={item.milestone} disabled={disabled||state.loading} onOpen={()=>requestPanel({repository:state.repository,number:item.number})}/>,author:<ProviderPeople names={item.author?[item.author]:null}/>,collaborators:<ProviderPeople names={item.assignees}/>,repository:<ProviderRepositorySummary repository={state.repository}/>,actions:<><button disabled={disabled||state.loading} onClick={()=>requestPanel({repository:state.repository,number:item.number})}>Open Issue Panel</button><details className="provider-more-actions"><summary>More</summary>{preferences&&<TaskPreferenceControls task={taskIdentity(state.repository,'issues',item.id)} pinned={preferenceState.pinned.has(taskIdentityKey(taskIdentity(state.repository,'issues',item.id)))} disabled={disabled||state.loading} change={preferenceState.change}/>}<CopyTextButton text={item.url} label={`Copy issue #${item.number} URL`} disabled={disabled||state.loading}/></details></>}))}/>}
  {state.nextPage!==null&&<button disabled={disabled||state.loading} onClick={()=>void feed.current?.load(state.repository,true)}>Load more issues</button>}
  </TaskGroup>
 </section>;
}
