import {useEffect,useRef,useState} from 'react';
import {api} from './api';
import type {NamedWorkspaceGroup,NamedRepository} from './namedWorkspaces';
import type {OpenRepositoryPath} from './workspaceWips';
import {FilterSelect} from './ui/FilterSelect';

export function WorkspaceLocalLinks({group,disabled,onLink}:{group:NamedWorkspaceGroup;disabled:boolean;onLink:(repository:NamedRepository,path:string|null)=>void}){
 const [paths,setPaths]=useState<OpenRepositoryPath[]>([]),[loading,setLoading]=useState(false),[error,setError]=useState('');
 const generation=useRef(0);
 const reload=async()=>{const request=++generation.current;setLoading(true);setError('');try{const result=await api.openRepositoryPaths();if(request===generation.current)setPaths(result);}catch{if(request===generation.current){setPaths([]);setError('Open repositories could not be loaded. Retry to choose a local repository.');}}finally{if(request===generation.current)setLoading(false);}};
 useEffect(()=>{void reload();return()=>{generation.current++;};},[]);
 const unique=[...new Map(paths.map(item=>[item.path,item])).values()];
 return <section aria-label="Workspace local links"><h4>Local repositories</h4>
  <p className="history-base">Link an open repository to show its local changes in WIPs.</p>
  <button disabled={disabled||loading} onClick={()=>void reload()}>{loading?'Loading open repositories…':'Refresh open repositories'}</button>
  {error&&<p role="alert">{error}</p>}
  {!loading&&!error&&!unique.length&&<p role="status">Open a repository, then refresh this list to link it.</p>}
  {!group.repositories.length&&<p>No repositories in this workspace. Use Edit workspace to add one.</p>}
  {group.repositories.map(repository=>{const key=repository.owner+'/'+repository.repo;const missing=repository.localPath&&!unique.some(item=>item.path===repository.localPath);return <div className="setting-row" key={key}><span>{key}</span><FilterSelect searchable label={`Local repository for ${key}`} placeholder="Not linked" disabled={disabled||loading||!!error} value={repository.localPath??''} options={[{value:'',label:'Not linked'},...unique.map(item=>({value:item.path,label:item.path})),...(missing?[{value:repository.localPath!,label:repository.localPath+' (not open)'}]:[])]} onChange={path=>{if(disabled||loading||error||(path&&!unique.some(item=>item.path===path)))return;onLink(repository,path||null);}}/></div>;})}
 </section>;
}
