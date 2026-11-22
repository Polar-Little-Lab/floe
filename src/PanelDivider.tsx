import {useRef,useState} from 'react';
const KEY='branchdesk-panel-widths';
type Widths={sidebar:number|null;details:number|null};
function load():Widths {
 try { const data=JSON.parse(localStorage.getItem(KEY)??'{}');return {sidebar:Number.isFinite(data.sidebar)&&data.sidebar>=140&&data.sidebar<=420?data.sidebar:null,details:Number.isFinite(data.details)&&data.details>=240&&data.details<=640?data.details:null}; }
 catch { return {sidebar:null,details:null}; }
}
export function usePanelWidths(){
 const [widths,setWidths]=useState(load);
 const [warning,setWarning]=useState('');
 const resize=(side:keyof Widths,width:number|null,persist:boolean)=>{
  const next={...widths,[side]:width};
  if(persist){try{localStorage.setItem(KEY,JSON.stringify(next));setWarning('');}catch{setWarning('Panel widths changed for this session but could not be saved.');}}
  setWidths(next);
 };
 const reset=()=>{
  const next:Widths={sidebar:null,details:null};setWidths(next);
  try{localStorage.setItem(KEY,JSON.stringify(next));setWarning('');}catch{setWarning('Default panel layout restored for this session but could not be saved.');}
 };
 return {widths,resize,reset,warning};
}
export function PanelDivider({side,width,onResize}:{side:keyof Widths;width:number|null;onResize:(width:number|null,persist:boolean)=>void}){
 const dragging=useRef(false);
 const min=side==='sidebar'?140:240, max=side==='sidebar'?420:640;
 const limit=(value:number,element:HTMLDivElement)=>{
  const workspace=element.parentElement;
  const other=workspace?.querySelector(side==='sidebar'?':scope > .details':':scope > .sidebar')?.getBoundingClientRect().width??0;
  const available=(workspace?.getBoundingClientRect().width??1200)-other-320;
  return Math.round(Math.max(min,Math.min(max,available,value)));
 };
 const position=(clientX:number,element:HTMLDivElement)=>{const rect=element.parentElement!.getBoundingClientRect();return limit(side==='sidebar'?clientX-rect.left:rect.right-clientX,element);};
 return <div className={`panel-divider panel-divider-${side}`} role="separator" aria-label={`Resize ${side} panel`} aria-orientation="vertical" aria-valuemin={min} aria-valuemax={max} aria-valuenow={width??(side==='sidebar'?220:420)} tabIndex={0} title="Drag to resize · double-click to reset"
  onPointerDown={e=>{if(e.button!==0)return;e.preventDefault();dragging.current=true;e.currentTarget.setPointerCapture(e.pointerId);}}
  onPointerMove={e=>{if(dragging.current)onResize(position(e.clientX,e.currentTarget),false);}}
  onPointerUp={e=>{if(dragging.current){dragging.current=false;onResize(position(e.clientX,e.currentTarget),true);e.currentTarget.releasePointerCapture(e.pointerId);}}}
  onPointerCancel={()=>{dragging.current=false;}} onLostPointerCapture={()=>{dragging.current=false;}}
  onDoubleClick={()=>onResize(null,true)}
  onKeyDown={e=>{if(e.key==='Home'){e.preventDefault();onResize(null,true);}else if(e.key==='ArrowLeft'||e.key==='ArrowRight'){e.preventDefault();const current=e.currentTarget.parentElement?.querySelector(side==='sidebar'?':scope > .sidebar':':scope > .details')?.getBoundingClientRect().width??width??min;const direction=(e.key==='ArrowRight'?1:-1)*(side==='sidebar'?1:-1);onResize(limit(current+direction*(e.shiftKey?40:10),e.currentTarget),true);}}}/>
}
