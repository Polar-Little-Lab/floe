/** Relative last activity, with the absolute provider timestamp always available. */
export function ActivityAge({timestamp,now}:{timestamp?:string|null;now:number}){
 const instant=timestamp&&/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z$/.test(timestamp)?Date.parse(timestamp):NaN;
 if(!Number.isFinite(instant))return <span title="Last activity unavailable">—</span>;
 const elapsed=now-instant;
 const minutes=Math.floor(Math.max(0,elapsed)/60000),hours=Math.floor(minutes/60),days=Math.floor(hours/24);
 const label=elapsed<0?'Future':days>=365?`${Math.floor(days/365)}y`:days>=30?`${Math.floor(days/30)}mo`:days>=7?`${Math.floor(days/7)}w`:days?`${days}d`:hours?`${hours}h`:minutes?`${minutes}m`:'now';
 return <time dateTime={timestamp!} title={`Last activity: ${new Date(instant).toLocaleString()}`} aria-label={`Last activity ${new Date(instant).toLocaleString()}`}>{label}</time>;
}
