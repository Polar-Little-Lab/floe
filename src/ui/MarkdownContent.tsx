import {createElement,useMemo,type ReactNode} from 'react';
import {CopyTextButton} from '../CopyTextButton';
import './markdown-content.css';
// Deliberately bounded Markdown subset. React escapes all text; no HTML or remote media.
function inline(text:string):ReactNode[]{
 const nodes:ReactNode[]=[];const pattern=/(`[^`\n]+`|\*\*[^*\n]+\*\*|~~[^~\n]+~~|\*[^*\n]+\*|!?\[[^\]\n]*\]\([^\s)]+\))/g;let cursor=0;
 for(const match of text.matchAll(pattern)){if(nodes.length>=400)break;const at=match.index!;nodes.push(text.slice(cursor,at));const token=match[0];
  if(token[0]==='`')nodes.push(<code key={at}>{token.slice(1,-1)}</code>);
  else if(token.startsWith('**'))nodes.push(<strong key={at}>{token.slice(2,-2)}</strong>);
  else if(token.startsWith('~~'))nodes.push(<del key={at}>{token.slice(2,-2)}</del>);
  else if(token[0]==='*')nodes.push(<em key={at}>{token.slice(1,-1)}</em>);
  else {const link=/^(!?)\[([^\]]*)\]\(([^)]+)\)$/.exec(token)!;let safe=false;try{const url=new URL(link[3]);safe=['https:','http:'].includes(url.protocol)&&!url.username&&!url.password;}catch{/* Relative and unsafe destinations stay literal. */}
   nodes.push(safe?<span className="markdown-link" key={at}>{link[1]?'Image: ':''}{link[2]||link[3]} <CopyTextButton text={link[3]} label={link[1]?'Copy image URL':'Copy link'} title={link[3]}/></span>:token);
  }cursor=at+token.length;
 }nodes.push(text.slice(cursor));return nodes;
}
const list=/^\s{0,3}([-+*]|\d+[.)])\s+(.+)$/;
const structural=(line:string)=>/^\s*$|^#{1,6}\s|^\s{0,3}(`{3,}|~{3,})|^>\s?|^\s{0,3}([-*_])\1\1+\s*$/.test(line)||list.test(line);
function blocks(text:string){const lines=text.replace(/\r\n?/g,'\n').split('\n');const nodes:ReactNode[]=[];
 for(let i=0;i<lines.length;){const key=i,line=lines[i];if(!line.trim()){i++;continue;}
  const fence=/^\s{0,3}(`{3,}|~{3,})(.*)$/.exec(line);
  if(fence){const code:string[]=[];const closing=new RegExp('^\\s{0,3}'+fence[1][0]+'{'+fence[1].length+',}\\s*$');i++;while(i<lines.length&&!closing.test(lines[i]))code.push(lines[i++]);if(i<lines.length)i++;nodes.push(<pre key={key}><code>{code.join('\n')}</code></pre>);continue;}
  const heading=/^(#{1,6})\s+(.+)$/.exec(line);if(heading){nodes.push(createElement(`h${heading[1].length}`,{key},inline(heading[2])));i++;continue;}
  if(/^\s{0,3}([-*_])\1\1+\s*$/.test(line)){nodes.push(<hr key={key}/>);i++;continue;}
  if(/^>/.test(line)){const quote:string[]=[];while(i<lines.length&&/^>/.test(lines[i]))quote.push(lines[i++].replace(/^>\s?/,''));nodes.push(<blockquote key={key}>{inline(quote.join('\n'))}</blockquote>);continue;}
  const item=list.exec(line);if(item){const ordered=/\d/.test(item[1]);const entries:ReactNode[]=[];const start=ordered?Number.parseInt(item[1],10):undefined;while(i<lines.length){const next=list.exec(lines[i]);if(!next||/\d/.test(next[1])!==ordered)break;const task=/^\[([ xX])\]\s+(.*)$/.exec(next[2]);entries.push(<li key={i}>{task?<><input type="checkbox" disabled checked={task[1].toLowerCase()==='x'} aria-label={task[2]}/>{inline(task[2])}</>:inline(next[2])}</li>);i++;}nodes.push(ordered?<ol key={key} start={start}>{entries}</ol>:<ul key={key}>{entries}</ul>);continue;}
  const paragraph=[line];i++;while(i<lines.length&&!structural(lines[i]))paragraph.push(lines[i++]);nodes.push(<p key={key}>{inline(paragraph.join('\n'))}</p>);
 }return nodes;
}
export function MarkdownContent({text}:{text:string}){const limited=text.length>65536;const rendered=useMemo(()=>blocks(text.slice(0,65536)),[text]);return <div className="markdown-content">{rendered}{limited&&<p role="status">Preview limited to 65,536 characters.</p>}</div>;}
