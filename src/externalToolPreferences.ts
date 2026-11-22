export type ExternalEditor='none'|'vscode';
const key='branchdesk.external-editor.v1';
let sessionChoice:ExternalEditor|undefined;
export function readExternalEditor():ExternalEditor{
 if(sessionChoice)return sessionChoice;
 try{return localStorage.getItem(key)==='vscode'?'vscode':'none';}catch{return 'none';}
}
export function saveExternalEditor(editor:ExternalEditor){
 sessionChoice=editor;
 localStorage.setItem(key,editor);
}
