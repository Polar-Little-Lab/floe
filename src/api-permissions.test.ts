/// <reference types="vite/client" />
import { expect, test } from 'vitest';
import ts from 'typescript';
import source from './api.ts?raw';
import build from '../src-tauri/build.rs?raw';
import handlerSource from '../src-tauri/src/main.rs?raw';
import capability from '../src-tauri/capabilities/main.json';

test('every native API invocation is registered and allowed only in the main window capability',()=>{
 const handlers=handlerSource.split('generate_handler![')[1]?.split(']')[0];
 const commands:string[]=[];
 const ast=ts.createSourceFile('api.ts',source,ts.ScriptTarget.Latest,true,ts.ScriptKind.TS);
 const visit=(node:ts.Node)=>{
  if(ts.isCallExpression(node)&&ts.isIdentifier(node.expression)&&node.expression.text==='invoke'){
   const command=node.arguments[0];
   expect(command&&ts.isStringLiteral(command),'native command must be a static string').toBe(true);
   if(command&&ts.isStringLiteral(command))commands.push(command.text);
  }
  ts.forEachChild(node,visit);
 };
 visit(ast);
 expect(commands).toContain('read_github_pull_requests');
 expect(commands.length).toBeGreaterThan(0);
 expect(capability.windows).toEqual(['main']);
 expect('remote' in capability ? capability.remote : undefined).toBeUndefined();
 for(const command of commands){
  expect(build,command).toContain(`"${command}"`);
  expect(handlers,command).toContain(command);
  expect(capability.permissions,command).toContain(`allow-${command.replaceAll('_','-')}`);
 }
});
