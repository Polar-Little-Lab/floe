import json,hashlib,struct,re,shutil
from pathlib import Path
app=Path(__file__).resolve().parents[1];ref=app.parent/'analysis/style-reference';rows=json.loads((ref/'asar-index.json').read_text());target=app/'public/vendor/gitkraken';manifest=[]
with open('/Applications/GitKraken.app/Contents/Resources/app.asar','rb')as f:
 h=struct.unpack('<4I',f.read(16));base=8+h[1]
 for r in rows:
  name=r['path']
  if not(name=='src/css/styles.css' or name.startswith('src/fonts/open-sans/') or name.startswith('src/images/')):continue
  if 'offset'not in r or r.get('unpacked'):continue
  out=target/name;out.parent.mkdir(parents=True,exist_ok=True);f.seek(base+int(r['offset']));data=f.read(r['size']);out.write_bytes(data)
  manifest.append({'path':name,'sha256':hashlib.sha256(data).hexdigest(),'bytes':len(data)})
icons=json.loads((ref/'icons-extracted.json').read_text());wanted={
'ArrowDown':'far:arrow-to-bottom','ArrowUp':'far:arrow-from-bottom','Undo2':'far:undo-alt','Redo2':'far:redo-alt','GitBranch':'far:code-branch','HardDriveDownload':'far:inbox-in','HardDriveUpload':'far:inbox-out','Terminal':'far:terminal','Search':'far:search','Settings':'far:cog','Bell':'far:bell','Folder':'far:folder','Rocket':'far:rocket','FileText':'fas:clipboard-list','List':'far:list','Bot':'fas:robot','Laptop':'far:laptop','Cloud':'far:cloud','TreePine':'far:tree','FlaskConical':'fas:flask','Users':'fas:users','Tag':'far:tag','Trash2':'far:trash-alt','Sparkles':'fas:sparkles','Pencil':'fas:pencil','Plus':'fas:plus','Minus':'fas:minus','ChevronDown':'far:chevron-down','ChevronRight':'far:chevron-right','ChevronLeft':'far:chevron-left','ArrowLeft':'far:arrow-left','Check':'fas:check','X':'fas:times','GitCommitHorizontal':'far:code-commit','Keyboard':'far:keyboard','ListFilter':'far:sort-alpha-down','Columns3':'far:folder-tree'}
used={key:icons[value]for key,value in wanted.items()};(app/'src/original-icons.json').write_text(json.dumps(used))
theme=json.loads((ref/'themes-extracted.json').read_text())[0]
(app/'src/original-theme.css').write_text(':root{'+''.join(k+':'+v+';'for k,v in theme.items() if isinstance(v,str))+'}\n')
(target/'provenance.json').write_text(json.dumps({'sourceVersion':'12.7.0','sourceArchive':str('/Applications/GitKraken.app/Contents/Resources/app.asar'),'visualFiles':manifest,'icons':wanted,'rendererCodeExecuted':False,'source':'static extraction; local UI study'},indent=2))
print('Original files copied:',len(manifest),'icons:',len(used))
