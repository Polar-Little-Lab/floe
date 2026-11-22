import {DiscardDraftDialog} from './ui/DiscardDraftDialog';
import {useEffect,useRef,useState} from 'react';
import type {Snapshot,Action} from './types';
import {validDefaultBranch} from './generalPreferences';
import {api,type GitflowSettings} from './api';
import {SettingsSection,SettingField} from './ui/WorkspaceScreen';
type Configuration=Extract<Action,{kind:'setGitflowConfiguration'}>['configuration'];
const defaults:Configuration={main:'main',develop:'develop',feature:'feature/',release:'release/',hotfix:'hotfix/',versionTag:''};
export function GitflowPreferencesPanel({repoId,available,onMutate,onOpen,onRun}:{repoId:string;available:boolean;onOpen?:()=>void;onRun:(task:()=>Promise<void>)=>Promise<void>;onMutate:(snapshot:Snapshot,action:Action)=>Promise<Snapshot>}){
 const [settings,setSettings]=useState<GitflowSettings|null>(null),[busy,setBusy]=useState(false),[error,setError]=useState('');
 const [needsReload,setNeedsReload]=useState(false);
 const [confirmReload,setConfirmReload]=useState(false);
 const [snapshot,setSnapshot]=useState<Snapshot|null>(null),[draft,setDraft]=useState<Configuration>(defaults),[message,setMessage]=useState('');
 const applySettings=(result:GitflowSettings)=>{setSettings(result);setDraft({main:result.main??defaults.main,develop:result.develop??defaults.develop,feature:result.feature??defaults.feature,release:result.release??defaults.release,hotfix:result.hotfix??defaults.hotfix,versionTag:result.versionTag??defaults.versionTag});};
 const generation=useRef(0),working=useRef(false);
 const load=async()=>{if(!available||working.current)return;const request=++generation.current;working.current=true;setBusy(true);setError('');try{await onRun(async()=>{const snapshot=await api.refresh(repoId);if(request!==generation.current)return;const result=await api.gitflowSettings(snapshot);if(request===generation.current){setSnapshot(snapshot);applySettings(result);setMessage('');setNeedsReload(false);}});}catch(error){if(request===generation.current)setError(String(error));}finally{if(request===generation.current){working.current=false;setBusy(false);}}};
 useEffect(()=>{let active=true;setSettings(null);setSnapshot(null);queueMicrotask(()=>{if(active)void load();});return()=>{active=false;generation.current++;working.current=false;};},[repoId,available]);
 const dirty=settings&&Object.keys(defaults).some(key=>draft[key as keyof Configuration]!== (settings[key as keyof Configuration]??defaults[key as keyof Configuration]));
 const branchesValid=validDefaultBranch(draft.main)&&validDefaultBranch(draft.develop)&&draft.main!==draft.develop;
 const prefixes=[draft.feature,draft.release,draft.hotfix];
 const valid=branchesValid&&new Set(prefixes).size===3&&prefixes.every(prefix=>prefix.length<=180&&validDefaultBranch(prefix+'example'))&&draft.versionTag.length<=180&&validDefaultBranch(draft.versionTag+'1.0.0');
 const missing=[draft.main,draft.develop].filter(name=>!snapshot?.branches.some(branch=>branch.name===name&&branch.oid));
 const save=async()=>{if(working.current||needsReload||!snapshot||!valid||missing.length)return;const request=generation.current;working.current=true;setBusy(true);setError('');setMessage('');try{await onRun(async()=>{const next=await onMutate(snapshot,{kind:'setGitflowConfiguration',configuration:draft});if(request!==generation.current)return;setSnapshot(next);setMessage('Gitflow configuration saved.');const result=await api.gitflowSettings(next);if(request===generation.current)applySettings(result);});}catch(error){if(request===generation.current){setNeedsReload(true);setError(String(error));}}finally{if(request===generation.current){working.current=false;setBusy(false);}}};
 const initialize=async()=>{
  if(working.current||needsReload||!snapshot?.headOid||!valid||snapshot.writeBlock||snapshot.recoveryPending)return;
  const request=generation.current;let current=snapshot;const completed:string[]=[];
  working.current=true;setBusy(true);setError('');setMessage('');
  const step=async(action:Action,label:string)=>{if(request!==generation.current)throw Error('Initialization stopped because the settings page changed.');current=await onMutate(current,action);completed.push(label);if(request!==generation.current)throw Error('Initialization stopped because the settings page changed.');setSnapshot(current);setMessage(completed.join(' → '));};
  try{await onRun(async()=>{
   let main=current.branches.find(branch=>branch.name===draft.main)?.oid;
   if(!main){main=current.headOid!;await step({kind:'createBranchAt',name:draft.main,oid:main},`Created ${draft.main}`);}
   if(!current.branches.some(branch=>branch.name===draft.develop&&branch.oid))await step({kind:'createBranchAt',name:draft.develop,oid:main},`Created ${draft.develop}`);
   await step({kind:'setGitflowConfiguration',configuration:draft},'Saved Gitflow configuration');
   if(current.branch!==draft.develop||current.detached)await step({kind:'checkout',name:draft.develop},`Checked out ${draft.develop}`);
   const result=await api.gitflowSettings(current);if(request===generation.current){applySettings(result);setMessage(`Gitflow initialized. ${completed.join(' → ')}`);}
  });}catch(error){if(request===generation.current){setNeedsReload(true);setError(String(error));setMessage(`Initialization stopped. ${completed.length?'Completed: '+completed.join(' → '):'No step was confirmed.'} Reload before trying again; completed steps are retained.`);}}
  finally{if(request===generation.current){working.current=false;setBusy(false);}}
 };
 const field=(key:keyof Configuration,label:string)=><SettingField label={label}><input aria-label={`Gitflow ${label}`} disabled={!snapshot||busy} maxLength={key==='main'||key==='develop'?200:180} value={draft[key]} onChange={event=>{setDraft({...draft,[key]:event.target.value});setMessage('');}} placeholder={settings?'Not configured':'—'}/>{key==='versionTag'&&draft.versionTag===''&&<p className="history-base">Version tags have no prefix.</p>}</SettingField>;
 return <section aria-label="Gitflow preferences">
  {confirmReload&&<DiscardDraftDialog onCancel={()=>setConfirmReload(false)} onDiscard={()=>{setConfirmReload(false);void load();}}/>}
  {!available&&<p>Open a repository to inspect Gitflow settings.</p>}
  <SettingsSection title="Branches">{field('main','Main/Master')}{field('develop','Develop')}</SettingsSection>
  <SettingsSection title="Prefixes">{field('feature','Feature')}{field('release','Release')}{field('hotfix','Hotfix')}{field('versionTag','Version Tag')}</SettingsSection>
  {settings?.warnings?.map(warning=><p role="status" key={warning}>{warning}</p>)}
  {settings?.missingBranches.length? <p role="alert">Configured branches are missing or invalid: {settings.missingBranches.join(', ')}</p>:null}
  {settings&&(!settings.main||!settings.develop)&&<p role="status">Gitflow branch configuration is incomplete.</p>}
  {error&&<p role="alert">{error}{settings?' Previously loaded settings are retained.':''}</p>}
  {snapshot&&!valid&&<p role="alert">Choose distinct valid branches and distinct valid prefixes.</p>}
  {snapshot&&missing.length>0&&<p role="status">Initialization will create missing branches: {missing.join(', ')}. Existing branches are retained.</p>}
  {message&&<p role="status">{message}</p>}
  <p className="history-base">Initialize creates missing main from the current HEAD, creates missing develop from main, saves configuration, then checks out develop. Each step requires native confirmation; completed steps remain if a later step is cancelled.</p>
  <button className="primary" disabled={!snapshot?.headOid||busy||needsReload||!valid||!!snapshot?.writeBlock||!!snapshot?.recoveryPending} onClick={()=>void initialize()}>Initialize Gitflow</button>
  <button className="primary" disabled={!snapshot||busy||needsReload||!!snapshot.writeBlock||!valid||missing.length>0} onClick={()=>void save()}>Save Gitflow configuration</button>
  <button disabled={!available||busy} onClick={()=>dirty?setConfirmReload(true):void load()}>{busy?'Loading Gitflow settings…':'Reload Gitflow settings'}</button>
  {onOpen&&<button disabled={!snapshot||busy||needsReload||!!dirty||!settings?.main||!settings?.develop||!!settings?.missingBranches.length||!!settings?.warnings?.length} onClick={onOpen}>Open Gitflow</button>}
  {dirty&&onOpen&&<p className="history-base">Save or reload your configuration changes before opening Gitflow.</p>}
  <p className="history-base">These values come from Git configuration. Saving changes Gitflow configuration only. Use Open Gitflow to start or finish a feature, release or hotfix with the saved configuration.</p>
 </section>;
}
