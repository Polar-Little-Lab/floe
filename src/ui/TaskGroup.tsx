import {createContext,useContext,useId,useState,type ReactNode} from 'react';
import {ChevronDown,ChevronRight} from '../OriginalIcons';
type Expansion={defaultExpanded:boolean;overrides:Record<string,boolean>};
type Controller={expanded:(id:string)=>boolean;toggle:(id:string)=>void;all:(expanded:boolean)=>void};
const Context=createContext<Controller|null>(null);
export function TaskGroupProvider({children}:{children:ReactNode}){
 const [state,setState]=useState<Expansion>({defaultExpanded:true,overrides:{}});
 const expanded=(id:string)=>state.overrides[id]??state.defaultExpanded;
 return <Context.Provider value={{expanded,toggle:id=>setState(previous=>({...previous,overrides:{...previous.overrides,[id]:!(previous.overrides[id]??previous.defaultExpanded)}})),all:defaultExpanded=>setState({defaultExpanded,overrides:{}})}}>{children}</Context.Provider>;
}
export function TaskGroupControls(){const controller=useContext(Context);if(!controller)return null;return <div className="task-group-controls" aria-label="Task group display"><button onClick={()=>controller.all(false)}><ChevronRight size={14}/>Collapse All</button><button onClick={()=>controller.all(true)}><ChevronDown size={14}/>Expand All</button></div>;}
export function TaskGroup({id,title,count,warning,children}:{id:string;title:string;count:number;warning?:boolean;children:ReactNode}){
 const controller=useContext(Context),bodyId=useId();const [local,setLocal]=useState(true);const expanded=controller?.expanded(id)??local;
 return <section className="task-group" aria-label={title}><h3><button aria-expanded={expanded} aria-controls={bodyId} onClick={()=>controller?controller.toggle(id):setLocal(!local)}>{expanded?<ChevronDown size={16}/>:<ChevronRight size={16}/>}<span>{title}</span><small>{count}</small>{warning&&<small className="task-group-warning">Needs attention</small>}</button></h3><div id={bodyId} hidden={!expanded}>{children}</div></section>;
}
