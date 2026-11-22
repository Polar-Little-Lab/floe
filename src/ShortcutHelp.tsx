import {useState} from 'react';
import {referenceMatcher} from './referenceFilter';
export function ShortcutHelp(){
 const [query,setQuery]=useState('');const mac=typeof navigator!=='undefined'&&navigator.platform.includes('Mac');const mod=mac?'⌘':'Ctrl';const alt=mac?'Option':'Alt';
 const shortcuts=[
  {action:'Search commits',keys:`${mod} K`,scope:'Main window; no modal open'},
  {action:'Command palette',keys:`${mod} Shift P`,scope:'Main window; idle and no modal open'},
  {action:'Focus branch filter',keys:`${mod} ${alt} F`,scope:'Main window; expands the local branch group'},
  {action:'Find in displayed content',keys:`${mod} F`,scope:'Focus inside Diff, File or Blame view'},
  {action:'Next / previous search match',keys:'Enter / Shift Enter',scope:'Diff, File or Blame search input'},
  {action:'Previous / next commit',keys:'↑ / ↓',scope:'Focused graph commit row; current filtered page'},
  {action:'First / last commit',keys:'Home / End',scope:'Focused graph commit row; current filtered page'},
  {action:'Move by visible page',keys:'Page Up / Page Down',scope:'Focused graph commit row'},
  {action:'Open context menu',keys:'Shift F10',scope:'Focused graph commit or local branch'},
  {action:'Move through menu items',keys:'↑ / ↓ / Home / End',scope:'Commit or branch context menu'},
  {action:'Select / run command',keys:'↑ / ↓, Enter',scope:'Command palette search input'},
  {action:'Collapse / expand folder',keys:'← / →',scope:'Focused changed-file folder in Tree view'},
  {action:'Resize panel',keys:'← / → (Shift: larger step)',scope:'Focused panel separator'},
  {action:'Reset panel width',keys:'Home',scope:'Focused panel separator'},
  {action:'Move between dialog controls',keys:'Tab / Shift Tab',scope:'Open modal; focus stays inside dialog'},
  {action:'Clear search / close view',keys:'Esc',scope:'Local search clears first; main close is disabled while busy'},
 ];const matches=referenceMatcher(query);const visible=shortcuts.filter(item=>matches(`${item.action} ${item.keys} ${item.scope}`));
 return <section className="shortcut-help" aria-label="Keyboard shortcut reference"><h3>Keyboard shortcuts · {mac?'macOS':'Windows / Linux'}</h3><input className="wide-input" aria-label="Filter keyboard shortcuts" placeholder="Filter shortcuts" value={query} onChange={event=>setQuery(event.target.value)}/><dl>{visible.map(item=><div className="shortcut-help-row" key={item.action}><dt>{item.action}<small>{item.scope}</small></dt><dd><kbd>{item.keys}</kbd></dd></div>)}</dl>{!visible.length&&<p>No matching shortcuts.</p>}<p>Some keys depend on the focused control. Custom key bindings are not available yet.</p></section>;
}
