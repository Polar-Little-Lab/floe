// A status hint only: never rejects, cancels, retries, or unlocks the real operation.
export function watchSlowOperation(notify:()=>void,delay=10_000):()=>void {
 const timer=setTimeout(notify,delay);
 return ()=>clearTimeout(timer);
}
