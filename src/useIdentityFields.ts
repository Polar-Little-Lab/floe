import {useState} from 'react';
import type {CommitIdentity} from './commitIdentity';
export function useIdentityFields(defaultIdentity:CommitIdentity|null|undefined){
 const [override,setOverride]=useState<CommitIdentity|null>(null);
 const identity=override??defaultIdentity??{name:'',email:''};
 return {author:identity.name,email:identity.email,setAuthor:(name:string)=>setOverride({...identity,name}),setEmail:(email:string)=>setOverride({...identity,email})};
}
