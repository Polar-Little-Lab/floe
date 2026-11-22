import {MetadataOptions,type LoadMetadataOptions} from './MetadataOptions';
// Controlled draft selector: selection does not submit provider mutations.
export function MetadataSelection({title,values,onChange,onLoad,disabled}:{title:string;values:string[];onChange:(values:string[])=>void;onLoad:LoadMetadataOptions;disabled:boolean}){
 return <section className="metadata-selection" aria-label={title}><h3>{title}</h3><div className="metadata-chips">{values.map(value=><span key={value}>{value}<button type="button" disabled={disabled} aria-label={`Remove ${title.toLowerCase()} ${value}`} onClick={()=>onChange(values.filter(item=>item!==value))}>×</button></span>)}</div><MetadataOptions title={title} selected={values} disabled={disabled} onLoad={onLoad} onSelect={value=>{if(!disabled&&values.length<100&&!values.includes(value))onChange([...values,value]);}}/></section>;
}
