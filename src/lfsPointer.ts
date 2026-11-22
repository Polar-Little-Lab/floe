export interface LfsPointer {oid:string;size:string;legacy:boolean;extraKeys:string[]}
// Read-only inspection of canonical v1 pointer text. Never resolves URLs or paths.
// Reference: https://github.com/git-lfs/git-lfs/blob/main/docs/spec.md
export function inspectLfsPointer(text:string):LfsPointer|null {
 if(text.length>=1024||!text.endsWith('\n')||text.includes('\r')||new TextEncoder().encode(text).length>=1024)return null;
 const lines=text.slice(0,-1).split('\n');
 const legacy=lines[0]==='version https://hawser.github.com/spec/v1';
 if(!legacy&&lines[0]!=='version https://git-lfs.github.com/spec/v1')return null;
 const values=new Map<string,string>();let previous='';
 for(const line of lines.slice(1)){const match=/^([a-z0-9.-]+) ([^\r\n]+)$/.exec(line);if(!match||match[1]==='version'||match[1]<=previous)return null;previous=match[1];values.set(match[1],match[2]);}
 const oid=/^sha256:([0-9a-f]{64})$/.exec(values.get('oid')??'');const size=values.get('size')??'';
 if(!oid||!/^(0|[1-9][0-9]*)$/.test(size))return null;
 return {oid:oid[1],size,legacy,extraKeys:[...values.keys()].filter(key=>key!=='oid'&&key!=='size')};
}
