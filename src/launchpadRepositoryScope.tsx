import {createContext,useContext,useState,type ReactNode} from 'react';
import type {RemoteInfo} from './types';
import {repositoryScope} from './providerPullRequests';
export type RepositorySuggestion={repository:string;remote:string};
export function githubRemoteSuggestions(remotes:RemoteInfo[]):RepositorySuggestion[]{
 const seen=new Set<string>(),result:RepositorySuggestion[]=[];
 for(const remote of remotes)for(const address of remote.fetchUrls){
  let path:string|undefined;
  const scp=/^git@github\.com:([^?#\s]+)$/.exec(address);
  if(scp)path=scp[1];
  else try{
   const url=new URL(address);
   if(url.hostname.toLowerCase()!=='github.com'||url.password||url.port||url.search||url.hash)continue;
   if((url.protocol==='https:'&&!url.username)||(url.protocol==='ssh:'&&url.username==='git'))path=url.pathname.slice(1);
  }catch{continue;}
  if(!path)continue;
  path=path.replace(/\/$/,'').replace(/\.git$/,'');
  try{const repository=repositoryScope(path).join('/'),key=repository.toLowerCase();if(!seen.has(key)){seen.add(key);result.push({repository,remote:remote.name});}}catch{continue;}
 }
 return result;
}
const Context=createContext<{scope:string;setScope:(value:string)=>void;suggestions:RepositorySuggestion[]}|null>(null);
export function LaunchpadRepositoryScope({suggestions,children,initialRepository}:{suggestions:RepositorySuggestion[];children:ReactNode;initialRepository?:string}){
 const [scope,setScope]=useState(()=>{if(initialRepository)try{return repositoryScope(initialRepository).join('/');}catch{/* Use a validated remote suggestion instead. */}return suggestions[0]?.repository??'';});
 return <Context.Provider value={{scope,setScope,suggestions}}>{children}</Context.Provider>;
}
export function useLaunchpadRepositoryScope(){
 const shared=useContext(Context);
 const [scope,setScope]=useState('');
 return shared??{scope,setScope,suggestions:[]};
}
