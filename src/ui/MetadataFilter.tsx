import type {ReactNode} from 'react';
import {FilterSelect} from './FilterSelect';
/** Null means no filter; JSON encoding keeps empty names distinct from no selection. */
export function MetadataFilter({label,allLabel,value,options,onChange,disabled=false,icon,missingLabel}:{label:string;allLabel:string;value:string|null;options:readonly {value:string;title:string}[];onChange:(value:string|null)=>void;disabled?:boolean;icon?:ReactNode;missingLabel?:string}){
 const choices=[{value:'',label:allLabel},...options.map(option=>({value:JSON.stringify(option.value),label:option.title}))];
 if(value!==null&&!options.some(option=>option.value===value))choices.push({value:JSON.stringify(value),label:`${missingLabel??value} (not in loaded results)`});
 return <FilterSelect searchable label={label} placeholder={allLabel} value={value===null?'':JSON.stringify(value)} options={choices} icon={icon} disabled={disabled} onChange={next=>onChange(next?JSON.parse(next) as string:null)}/>;
}
