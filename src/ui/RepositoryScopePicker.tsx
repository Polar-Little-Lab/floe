import type {RepositorySuggestion} from '../launchpadRepositoryScope';
export function RepositoryScopePicker({suggestions,value='',label,placeholder='Choose repository…',disabled=false,onSelect}:{suggestions:RepositorySuggestion[];value?:string;label:string;placeholder?:string;disabled?:boolean;onSelect:(repository:string)=>void}){
 if(!suggestions.length)return null;
 return <select aria-label={label} disabled={disabled} value={suggestions.some(item=>item.repository===value)?value:''} onChange={event=>{if(event.target.value)onSelect(event.target.value);}}><option value="">{placeholder}</option>{suggestions.map(item=><option key={item.repository} value={item.repository}>{item.remote}: {item.repository}</option>)}</select>;
}
