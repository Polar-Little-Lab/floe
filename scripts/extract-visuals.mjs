import ts from 'typescript';
import fs from 'node:fs';
import path from 'node:path';
const sourceRoot=path.resolve('../analysis/style-reference');
const source=fs.readFileSync(path.join(sourceRoot,'src/render/static/entryPoints/main/render.bundle.js'),'utf8');
const tree=ts.createSourceFile('renderer.js',source,ts.ScriptTarget.Latest,true,ts.ScriptKind.JS);
const icons={};const themes=[];
function literal(node,vars={}) {
 if(!node)return undefined;
 if(ts.isStringLiteral(node)||ts.isNumericLiteral(node))return ts.isNumericLiteral(node)?Number(node.text):node.text;
 if(ts.isIdentifier(node))return vars[node.text];
 if(ts.isArrayLiteralExpression(node))return node.elements.map(n=>literal(n,vars));
 if(ts.isObjectLiteralExpression(node)){const out={};for(const p of node.properties){if(!ts.isPropertyAssignment(p))return undefined;out[p.name.text]=literal(p.initializer,vars);}return out;}
 return undefined;
}
function visit(node){
 if(ts.isObjectLiteralExpression(node)&&node.properties.some(p=>p.name?.text==='--app__bg0')){const value=literal(node);if(value)themes.push(value);}
 if(ts.isMethodDeclaration(node)&&node.body){
  const vars={};const definitions=[];
  function scan(n){
   if(ts.isVariableDeclaration(n)&&ts.isIdentifier(n.name)){const value=literal(n.initializer,vars);if(value!==undefined)vars[n.name.text]=value;}
   if(ts.isBinaryExpression(n)&&ts.isPropertyAccessExpression(n.left)&&n.left.name.text==='definition'){const value=literal(n.right,vars);if(value?.prefix&&value.iconName&&value.icon)definitions.push(value);}
   ts.forEachChild(n,scan);
  }
  scan(node.body);
  for(const value of definitions){if(value.icon.every(v=>v!==undefined))icons[value.prefix+':'+value.iconName]=value.icon;}
 }
 ts.forEachChild(node,visit);
}
visit(tree);
fs.writeFileSync(path.join(sourceRoot,'icons-extracted.json'),JSON.stringify(icons,null,2));
fs.writeFileSync(path.join(sourceRoot,'themes-extracted.json'),JSON.stringify(themes,null,2));
console.log({icons:Object.keys(icons).length,themes:themes.length});
console.log(Object.keys(icons).filter(k=>/undo|redo|arrow|branch|terminal|list|laptop|cloud|pencil|folder|rocket|cog|magic|spark|tree|archive|sort|commit|tray/.test(k)).join('\n'));
