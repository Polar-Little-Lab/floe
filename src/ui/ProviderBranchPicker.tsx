import {useRef} from 'react';
import {MetadataOptions,type LoadMetadataOptions} from './MetadataOptions';
// Same branch selector composition for both sides of the create-PR form.
export function ProviderBranchPicker({label,value,onChange,onLoad,disabled}:{label:string;value:string;onChange:(value:string)=>void;onLoad:LoadMetadataOptions;disabled:boolean}){
 const container=useRef<HTMLDivElement>(null);
 return <div className="provider-branch-picker" ref={container} role="group" aria-label={label}><span>Branch</span><MetadataOptions title={label} summaryLabel={value||'Choose branch'} selected={value?[value]:[]} disabled={disabled} onLoad={onLoad} onSelect={branch=>{if(disabled)return;onChange(branch);const dropdown=container.current?.querySelector('details.metadata-option-picker');if(dropdown instanceof HTMLDetailsElement){dropdown.open=false;dropdown.querySelector('summary')?.focus();}}}/><details className="provider-branch-manual"><summary>Enter branch name</summary><input aria-label={label} value={value} maxLength={1024} disabled={disabled} onChange={event=>onChange(event.target.value)}/></details></div>;
}
