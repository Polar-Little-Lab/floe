import {ProviderItemSummary,ProviderRepositorySummary,ProviderPeople} from './ui/ProviderItemSummary';
import {MetadataFilter} from './ui/MetadataFilter';
import {FilterSelect} from './ui/FilterSelect';
import {Tag,FileText} from './OriginalIcons';
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
import {PullRequestFeed,filterPullRequests,type PullDraftFilter,type PullRequestState} from './providerPullRequests';
export function LaunchpadPullRequests({disabled=false,preferences}:{disabled?:boolean;preferences?:TaskPreferences}){
 const {scope,setScope}=useLaunchpadRepositoryScope();
 const {panel,setPanel,requestPanel,registerNavigationGuard}=useProviderPanel<{repository:string;number?:number}>(previous=>{void feed.current?.load(previous.repository);});
 const [label,setLabel]=useState<string|null>(null);const [milestone,setMilestone]=useState<string|null>(null);
 const [query,setQuery]=useTaskSearch();const [draft,setDraft]=useState<PullDraftFilter>('all');
 const [state,setState]=useState<PullRequestState>({repository:'',items:[],nextPage:null,loading:false,error:'',loaded:false});
 const feed=useRef<PullRequestFeed|null>(null);
 if(!feed.current)feed.current=new PullRequestFeed(api.readGithubPullRequests,setState);
 useEffect(()=>()=>feed.current?.dispose(),[]);
 const preferenceState=useTaskPreferences(preferences);
 const visible=visiblePreferenceItems(filterPullRequests(state.items,query,draft,label,milestone),state.repository,'pulls',preferenceState);
 const labels=[...new Set(state.items.flatMap(item=>item.labels??[]))].sort((a,b)=>a.localeCompare(b));
 const milestones=[...new Map(state.items.flatMap(item=>item.milestone?[[item.milestone.number,item.milestone] as const]:[])).values()];
 useEffect(()=>{setLabel(null);setMilestone(null);},[state.repository]);
 return <section aria-label="GitHub pull requests">
  <ProviderFeedHeader title="Pull requests" scope={scope} inputLabel="GitHub repository" loadLabel="Load pull requests" createLabel="Create Pull Request" disabled={disabled||state.loading} disabledReason={state.loading?'Loading pull requests…':undefined} onScope={setScope} onLoad={()=>void feed.current?.load(scope)} onCreate={()=>requestPanel({repository:scope.trim()})}><p>Public GitHub repositories. Filters apply to loaded results. Pins and snoozes are saved on this device.</p></ProviderFeedHeader>
  {panel&&<ProviderItemPanel registerNavigationGuard={registerNavigationGuard} repository={panel.repository} number={panel.number} pull={true} onClose={refresh=>{const repository=panel.repository;setPanel(null);if(refresh)void feed.current?.load(repository);}}/>}
  {state.loading&&<p role="status">Loading pull requests…</p>}
  {preferenceState.error&&<p role="alert">{preferenceState.error} Reload workspaces before retrying.</p>}
  {state.error&&<p role="alert">{state.error}</p>}
  <ProviderFilterBar scope="pulls" label="Pull requests" query={query} onQuery={setQuery} onRefresh={()=>void feed.current?.load(state.repository)} onReset={()=>{setQuery('');setDraft('all');setLabel(null);setMilestone(null);}} active={!!query||draft!=='all'||label!==null||milestone!==null} disabled={disabled||state.loading} loaded={state.loaded} summary={state.loaded?`${state.repository} · ${visible.length} of ${state.items.length} loaded${state.nextPage!==null?' · more available':''}`:undefined}>
   <FilterSelect label="Filter pull request draft status" placeholder="All draft states" value={draft} disabled={!state.loaded||disabled||state.loading} options={[{value:'all',label:'All draft states'},{value:'draft',label:'Drafts'},{value:'not-draft',label:'Not drafts'},{value:'unknown',label:'Unknown draft state'}]} onChange={value=>setDraft(value as PullDraftFilter)}/>
   <MetadataFilter label="Filter pull request label" allLabel="All labels" icon={<Tag size={15}/>} value={label} options={labels.map(value=>({value,title:value}))} onChange={setLabel} disabled={!state.loaded||disabled||state.loading}/>
   <MetadataFilter label="Filter pull request milestone" allLabel="All milestones" icon={<FileText size={15}/>} value={milestone} options={milestones.map(value=>({value:value.number,title:`${value.title} (#${value.number})`}))} missingLabel={`Milestone #${milestone}`} onChange={setMilestone} disabled={!state.loaded||disabled||state.loading}/>
  </ProviderFilterBar>
  {state.loaded&&!state.loading&&!state.items.length&&<p>No open pull requests.</p>}
  {state.loaded&&state.items.length>0&&!visible.length&&<p>No matching loaded pull requests.</p>}
  <TaskGroup id={`single:pulls:${state.repository}`} title={state.repository?`${state.repository} · Pull requests`:'Pull requests'} count={visible.length} warning={!!state.error}>
  {!visible.length&&<ProviderTable label="Pull Requests" rows={[]} empty={<><h3>{state.loaded?'No matching pull requests':'Your pull requests appear here'}</h3><p>{state.loaded?'Change the filters or choose another repository.':'Select a workspace or enter owner/repository above to load pull requests.'}</p></>}/> }
  {visible.length>0&&<ProviderTable label="Pull requests" rows={visible.map(item=>({id:item.id,updatedAt:item.updatedAt,selected:panel?.number===item.number&&panel.repository.toLowerCase()===state.repository.toLowerCase(),onOpen:disabled||state.loading?undefined:()=>requestPanel({repository:state.repository,number:item.number}),status:item.draft===true?'Draft':item.draft===null?'Draft status unknown':'Open',item:<ProviderItemSummary number={item.number} title={item.title} labels={item.labels} milestone={item.milestone} disabled={disabled||state.loading} onOpen={()=>requestPanel({repository:state.repository,number:item.number})}/>,author:<ProviderPeople names={item.author?[item.author]:null}/>,collaborators:<ProviderPeople names={item.requestedReviewers}/>,repository:<ProviderRepositorySummary repository={state.repository} head={item.headBranch} base={item.baseBranch}/>,actions:<><button disabled={disabled||state.loading} onClick={()=>requestPanel({repository:state.repository,number:item.number})}>Open Pull Request Panel</button><details className="provider-more-actions"><summary>More</summary>{preferences&&<TaskPreferenceControls task={taskIdentity(state.repository,'pulls',item.id)} pinned={preferenceState.pinned.has(taskIdentityKey(taskIdentity(state.repository,'pulls',item.id)))} disabled={disabled||state.loading} change={preferenceState.change}/>}<CopyTextButton text={item.url} label={`Copy pull request #${item.number} URL`} disabled={disabled||state.loading}/></details></>}))}/>}
  {state.nextPage!==null&&<button disabled={disabled||state.loading} onClick={()=>void feed.current?.load(state.repository,true)}>Load more pull requests</button>}
  </TaskGroup>
 </section>;
}
