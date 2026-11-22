import type {PushRecord} from './types';
export function PushResultBanner({record,busy,onCheck,onAcknowledge}:{record:PushRecord;busy:boolean;onCheck:()=>void;onAcknowledge:()=>void}) {
 const pending=record.status==='pending';
 const labels={pending:'Push result unconfirmed. No automatic retry.',accepted:'Server accepted Push.',rejected:'Server rejected Push.',notSent:'Push stopped before sending.'};
 const observations={targetPresent:'The remote currently points to the selected commit.',previousTarget:'The remote currently points to the previous commit.',differentTarget:'The remote currently points to a different commit.',missing:'The remote branch was not advertised.'};
 return <div className="error-banner" role="status"><strong>Push · {record.remote}/{record.destination.replace('refs/heads/','')}</strong><p>{labels[record.status]}</p>{record.detail&&<p>{record.detail}</p>}{record.observation&&<p>{observations[record.observation.kind]} {record.observation.oid?.slice(0,12)} · Checked {new Date(record.observation.checkedAt*1000).toLocaleString()}. This observation does not prove whether the earlier Push succeeded.</p>}{pending&&<button disabled={busy} onClick={onCheck}>Check remote result</button>}{(!pending||record.observation)&&<button disabled={busy} onClick={onAcknowledge}>{pending?'Archive unconfirmed result':'Close confirmed Push result'}</button>}</div>;
}
