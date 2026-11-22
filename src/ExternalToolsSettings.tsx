import {useEffect,useRef,useState} from 'react';
import {api,native} from './api';
import {SettingField,SettingsSection} from './ui/WorkspaceScreen';
import {readExternalEditor,saveExternalEditor,type ExternalEditor} from './externalToolPreferences';
export function ExternalToolsSettings({repoId,repositoryName,disabled}:{repoId:string;repositoryName:string;disabled:boolean}){
 const [editor,setEditor]=useState<ExternalEditor>(readExternalEditor);
 const [tools,setTools]=useState<Awaited<ReturnType<typeof api.availableExternalEditors>>|null>(null);
 const [busy,setBusy]=useState(false),[error,setError]=useState(''),[message,setMessage]=useState('');
 const running=useRef(false);
 useEffect(()=>{let active=true;if(native)void api.availableExternalEditors().then(value=>{if(active)setTools(value);}).catch(error=>{if(active)setError(String(error));});return()=>{active=false;};},[]);
 const select=(value:ExternalEditor)=>{setEditor(value);setMessage('');try{saveExternalEditor(value);setError('');}catch{setError('Editor changed for this session, but could not be saved.');}};
 const run=async(action:()=>Promise<unknown>,success:string)=>{if(running.current||disabled||!native)return;running.current=true;setBusy(true);setError('');setMessage('');try{await action();setMessage(success);}catch(error){setError(String(error));}finally{running.current=false;setBusy(false);}};
 const unavailable=disabled||busy||!native;
 return <>
  <SettingsSection title="Merge Tool"><SettingField label="External Merge Tool" hint="External merge tool execution is not implemented. Resolve conflicts in the repository conflict view."><select aria-label="External Merge Tool" disabled value="internal"><option value="internal">Built-in conflict resolution</option></select></SettingField></SettingsSection>
  <SettingsSection title="Diff Tool"><SettingField label="External Diff Tool" hint="External diff tool execution is not implemented. File comparisons use the built-in diff view."><select aria-label="External Diff Tool" disabled value="internal"><option value="internal">Built-in diff viewer</option></select></SettingField></SettingsSection>
  <SettingsSection title="Editor"><SettingField label="External Editor" hint="Choose an installed editor to open the current repository. Its own workspace settings apply."><select aria-label="External Editor" value={editor} disabled={busy} onChange={event=>select(event.target.value as ExternalEditor)}><option value="none">&lt;None&gt;</option><option value="vscode">Visual Studio Code{tools&&!tools.some(tool=>tool.id==='vscode'&&tool.available)?' (not found)':''}</option></select><div className="setting-row"><button disabled={unavailable||editor==='none'||!tools?.some(tool=>tool.id===editor&&tool.available)} onClick={()=>void run(()=>api.openEditor(repoId,'vscode'),'Editor launch requested.')}>Open repository in editor</button><button disabled={busy||!native} onClick={()=>{setError('');void api.availableExternalEditors().then(setTools).catch(error=>setError(String(error)));}}>Refresh installed editors</button></div></SettingField></SettingsSection>
  <SettingsSection title="Terminal"><SettingField label="External Terminal" hint="Opens the current repository folder in the system terminal."><select aria-label="External Terminal" value="system" disabled><option value="system">{navigator.platform.includes('Mac')?'Terminal.app':navigator.platform.includes('Win')?'Command Prompt':'System terminal'}</option></select><p><button disabled={unavailable} onClick={()=>void run(()=>api.openTerminal(repoId),'Terminal launch requested.')}>Open repository in terminal</button></p></SettingField></SettingsSection>
  <p className="history-base">{disabled?'Open a repository to use external tools.':`Current repository: ${repositoryName}`}</p>
  {!native&&<p>External applications are available in the desktop app.</p>}
  {busy&&<p role="status">Opening external application…</p>}{message&&<p role="status">{message}</p>}{error&&<p role="alert">{error}</p>}
 </>;
}
