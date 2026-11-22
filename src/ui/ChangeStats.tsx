export type ChangeStatistics={changedFiles?:number|null;additions?:number|null;deletions?:number|null;commits?:number|null};
const known=(value:number|null|undefined):value is number=>typeof value==='number'&&Number.isSafeInteger(value)&&value>=0;
export function ChangeStats({value}:{value:ChangeStatistics}){
 return <div className="change-statistics" aria-label="Pull request change statistics"><span>{known(value.changedFiles)?`${value.changedFiles.toLocaleString()} ${value.changedFiles===1?'file':'files'} changed`:'Changed file count unavailable'}</span>{known(value.commits)&&<span>{value.commits.toLocaleString()} {value.commits===1?'commit':'commits'}</span>}{known(value.additions)&&<span className="change-additions" aria-label={`${value.additions} lines added`}>+{value.additions.toLocaleString()}</span>}{known(value.deletions)&&<span className="change-deletions" aria-label={`${value.deletions} lines deleted`}>−{value.deletions.toLocaleString()}</span>}</div>;
}
