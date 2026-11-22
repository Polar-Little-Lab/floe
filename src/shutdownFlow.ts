export async function flushBeforeExit(flush:()=>Promise<void>,finish:(saved:boolean)=>Promise<void>):Promise<void>{
 try {await flush();} catch(error){await finish(false);throw error;}
 await finish(true);
}
