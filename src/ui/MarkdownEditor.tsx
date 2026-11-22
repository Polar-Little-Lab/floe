import {useId,useRef,useState} from 'react';
import {MarkdownContent} from './MarkdownContent';
type Format='bold'|'italic'|'strike'|'heading'|'quote'|'bullets'|'numbers'|'link'|'code'|'block';
const formats:readonly {id:Format;label:string;glyph:string}[]=[{id:'bold',label:'Bold',glyph:'B'},{id:'italic',label:'Italic',glyph:'I'},{id:'strike',label:'Strikethrough',glyph:'S̶'},{id:'heading',label:'Heading',glyph:'H'},{id:'numbers',label:'Numbered list',glyph:'1.'},{id:'bullets',label:'Bulleted list',glyph:'•'},{id:'link',label:'Insert link',glyph:'Link'},{id:'quote',label:'Quote',glyph:'❞'},{id:'code',label:'Inline code',glyph:'</>'},{id:'block',label:'Code block',glyph:'{ }'}];
/** Shared draft editor. Preview never submits; formatting changes only the controlled draft. */
export function MarkdownEditor({label,value,onChange,disabled=false,rows=6,maxLength=65536}:{label:string;value:string;onChange:(value:string)=>void;disabled?:boolean;rows?:number;maxLength?:number}){
 const id=useId(),textarea=useRef<HTMLTextAreaElement>(null),[preview,setPreview]=useState(false),[message,setMessage]=useState('');
 const format=(kind:Format)=>{
  const input=textarea.current;if(!input||disabled||preview||input.matches(':disabled'))return;
  let start=input.selectionStart,end=input.selectionEnd;const selected=value.slice(start,end);let replacement='',selectionStart=0,selectionEnd=0;
  if(['heading','quote','bullets','numbers'].includes(kind)){
   start=value.lastIndexOf('\n',Math.max(0,start-1))+1;if(input.selectionStart===0)start=0;
   const next=value.indexOf('\n',end);end=next<0?value.length:next;
   replacement=value.slice(start,end).split('\n').map((line,index)=>(kind==='heading'?'## ':kind==='quote'?'> ':kind==='bullets'?'- ':`${index+1}. `)+line).join('\n');selectionEnd=replacement.length;
  }else if(kind==='link'){
   const text=selected||'link text';replacement=`[${text}](https://)`;selectionStart=text.length+3;selectionEnd=replacement.length-1;
  }else if(kind==='block'||kind==='code'&&(selected.includes('\n')||selected.includes('`'))){
   const fence='`'.repeat(Math.max(3,...(selected.match(/`+/g)??[]).map(run=>run.length+1)));
   const prefix=(start>0&&value[start-1]!=='\n'?'\n':'')+fence+'\n';const code=selected||'code';replacement=prefix+code+'\n'+fence+(end<value.length&&value[end]!=='\n'?'\n':'');selectionStart=prefix.length;selectionEnd=prefix.length+code.length;
  }else{
   const marker=kind==='bold'?'**':kind==='italic'?'*':kind==='strike'?'~~':'`';const text=selected||(kind==='code'?'code':'text');replacement=marker+text+marker;selectionStart=marker.length;selectionEnd=marker.length+text.length;
  }
  const next=value.slice(0,start)+replacement+value.slice(end);if(next.length>maxLength){setMessage('Formatting would exceed the character limit.');return;}
  setMessage('');onChange(next);requestAnimationFrame(()=>{if(textarea.current!==input)return;input.focus();input.setSelectionRange(start+selectionStart,start+selectionEnd);});
 };
 return <section className="markdown-editor" aria-label={`${label} editor`}>
  <div className="markdown-editor-heading"><label htmlFor={id}>{label}</label><div role="group" aria-label={`${label} mode`}><button type="button" disabled={disabled} aria-pressed={!preview} onClick={()=>setPreview(false)}>Write</button><button type="button" disabled={disabled} aria-pressed={preview} onClick={()=>setPreview(true)}>Preview</button></div></div>
  <div hidden={preview}><div className="markdown-format-toolbar" role="group" aria-label={`${label} formatting`}>{formats.map(item=><button key={item.id} type="button" disabled={disabled} title={item.label} aria-label={`${item.label}: ${label}`} onMouseDown={event=>event.preventDefault()} onClick={()=>format(item.id)}>{item.glyph}</button>)}</div><textarea ref={textarea} id={id} aria-label={label} className="wide-input" rows={rows} maxLength={maxLength} value={value} disabled={disabled} onChange={event=>{setMessage('');onChange(event.target.value);}} onKeyDown={event=>{if(event.nativeEvent.isComposing||event.altKey||(!event.metaKey&&!event.ctrlKey))return;const key=event.key.toLowerCase();if(key==='b'||key==='i'){event.preventDefault();event.stopPropagation();format(key==='b'?'bold':'italic');}}}/></div>
  {preview&&<div className="markdown-editor-preview" role="region" aria-label={`${label} preview`}>{value.trim()?<MarkdownContent text={value}/>:<p className="history-base">Nothing to preview.</p>}</div>}
  {message&&<p role="status">{message}</p>}<small className="history-base">Markdown supported · {value.length.toLocaleString()}/{maxLength.toLocaleString()}</small>
 </section>;
}
