/** Literal AND search over reference labels and object IDs. */
export function referenceMatcher(query:string):(text:string)=>boolean {
 const terms=query.trim().toLocaleLowerCase().split(/\s+/).filter(Boolean);
 return text=>{const value=text.toLocaleLowerCase();return terms.every(term=>value.includes(term));};
}
