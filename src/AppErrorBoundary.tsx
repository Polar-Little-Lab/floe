import {Component,type ErrorInfo,type ReactNode} from 'react';
import {CopyTextButton} from './CopyTextButton';
type State={message:string|null;details:string;acknowledged:boolean};
export class AppErrorBoundary extends Component<{children:ReactNode},State>{
 state:State={message:null,details:'',acknowledged:false};
 static getDerivedStateFromError(error:unknown):Partial<State>{return {message:(error instanceof Error?error.message:String(error)).slice(0,4000)||'Unknown interface error',acknowledged:false};}
 componentDidCatch(error:Error,info:ErrorInfo){this.setState({details:`${error.stack??error.message}\n${info.componentStack??''}`.slice(0,16000)});}
 render(){
  if(this.state.message===null)return this.props.children;
  return <main className="app-error-screen"><section role="alert" aria-label="Interface error"><h1>The interface could not be displayed</h1><p>A screen encountered an unexpected error. No recovery Git command has been run.</p><pre>{this.state.message}</pre><p>A Git operation already started may still be running. Reloading the interface does not cancel or undo it. After reloading, refresh the repository and check its state before repeating an operation.</p><label><input type="checkbox" checked={this.state.acknowledged} onChange={event=>this.setState({acknowledged:event.target.checked})}/>I understand that unsaved text and the current view may be lost.</label><div className="setting-row"><button disabled={!this.state.acknowledged} onClick={()=>window.location.reload()}>Reload interface</button><CopyTextButton text={this.state.details||this.state.message} label="Copy error details"/></div><details><summary>Error details</summary><p>Details may include local paths. Review before sharing.</p><pre>{this.state.details||this.state.message}</pre></details></section></main>;
 }
}
