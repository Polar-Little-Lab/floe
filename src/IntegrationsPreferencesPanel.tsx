import {useEffect,useRef,useState} from 'react';
import {ScreenNavigation,SettingField,SettingsSection} from './ui/WorkspaceScreen';
import {repositoryScope} from './providerPullRequests';
import {FilterSelect} from './ui/FilterSelect';
import type {RepositorySuggestion} from './launchpadRepositoryScope';
import type {MetadataOptionsPage} from './ui/MetadataOptions';
const providers=['GitHub','GitHub Enterprise Server','GitLab','GitLab Self-Managed','Bitbucket','Bitbucket Server','Azure DevOps','Jira Cloud','Jira Server','Trello'];
export function IntegrationsPreferencesPanel({suggestions,disabled,onCheck,onForget,onLaunchpad,launchpadOpen=false}:{suggestions:RepositorySuggestion[];disabled:boolean;launchpadOpen?:boolean;onLaunchpad?:(repository:string)=>void;onForget?:(owner:string,repo:string)=>Promise<boolean>;onCheck:(owner:string,repo:string)=>Promise<MetadataOptionsPage|null>}){
 const [repository,setRepository]=useState(suggestions[0]?.repository??''),[busy,setBusy]=useState(false),[message,setMessage]=useState(''),[error,setError]=useState('');
 const working=useRef(false),mounted=useRef(true);
 useEffect(()=>{mounted.current=true;return()=>{mounted.current=false;};},[]);
 let parsed:string[]|null=null;try{parsed=repositoryScope(repository.trim());}catch{/* Explain only after input. */}
 const change=(value:string)=>{setRepository(value);setMessage('');setError('');};
 const check=async()=>{if(disabled||working.current||!parsed)return;const [owner,repo]=parsed;working.current=true;setBusy(true);setMessage('');setError('');try{const result=await onCheck(owner,repo);if(mounted.current)setMessage(result?`Access confirmed for ${owner}/${repo}. Read ${result.items.length} branches${result.nextPage?' on the first page':''}. This does not verify permission to create or merge pull requests.`:'Access check cancelled. No connection status was changed.');}catch(reason){if(mounted.current)setError(String(reason));}finally{working.current=false;if(mounted.current)setBusy(false);}};
 const forget=async()=>{if(disabled||working.current||!parsed||!onForget)return;const [owner,repo]=parsed;working.current=true;setBusy(true);setMessage('');setError('');try{const removed=await onForget(owner,repo);if(mounted.current)setMessage(removed?`No saved credential remains for ${owner}/${repo}. The token was not revoked on GitHub.`:'Credential removal cancelled.');}catch(reason){if(mounted.current)setError(String(reason));}finally{working.current=false;if(mounted.current)setBusy(false);}};
 return <div className="integrations-preferences">
  <ScreenNavigation label="Integration providers" vertical value="GitHub" items={providers.map(label=>({id:label,label,disabled:label!=='GitHub',unavailableReason:label!=='GitHub'?'This provider integration is not implemented.':undefined}))} onChange={()=>{}}/>
 <section aria-label="GitHub integration">
 <SettingsSection title="GitHub">
 <p>Check access to a GitHub.com repository used by Launchpad pull requests and issues.</p>{suggestions.length>0&&<SettingField label="Repository remote">
 <FilterSelect label="GitHub repository remote" searchable placeholder="Choose a repository" value={repository} options={suggestions.map(item=>({value:item.repository,label:`${item.repository} (${item.remote})`}))} disabled={disabled||busy} onChange={change}/>
 </SettingField>}<SettingField label="Repository">
 <input aria-label="GitHub repository" value={repository} placeholder="owner/repository" maxLength={300} disabled={disabled||busy} onChange={event=>change(event.target.value)}/>{repository&&!parsed&&<p role="alert">Enter a GitHub repository as owner/repository.</p>}</SettingField>
 <SettingField label="Authentication">
 <p>Credentials are requested in the native access dialog for this repository. Tokens are not entered or displayed in this settings page.</p>
 <button disabled={disabled||busy||!parsed} onClick={()=>void check()}>{busy?'Working…':'Check GitHub access'}</button>{onForget&&<button disabled={disabled||busy||!parsed} onClick={()=>void forget()}>Forget saved GitHub credential</button>}</SettingField>{message&&<p role="status">{message}</p>}{error&&<p role="alert">{error}</p>}</SettingsSection>
 {onLaunchpad&&<SettingField label="Launchpad"><button disabled={disabled||busy||!parsed} onClick={()=>{if(parsed)onLaunchpad(parsed.join('/'));}}>{launchpadOpen?'Return to Launchpad':'Open Launchpad'}</button><p>{launchpadOpen?'Return to the existing view and retain its drafts.':'Prefill the personal pull request and issue repository field. Your default view and saved workspace selection are preserved.'}</p></SettingField>}
 </section>
 </div>;
}
