import {expect,test} from 'vitest';
import {spawnSync} from 'node:child_process';
import {validPushDestination} from './PushDestinationDialog';
test('destination usability rules agree with Git ref validation across ordinary and hostile names',()=>{
 const names=['main','release/v2','한글/브랜치','@','feature/a+b','a.lock/x','x.locked','a..b','@{1}','foo bar','x:y','-force','/main','main/','a//b','.hidden','a/.hidden','a.lock','foo\\bar','foo?','foo[','main.','bad\nname'];
 for(const name of names){const git=spawnSync('git',['check-ref-format',`refs/heads/${name}`],{stdio:'ignore'});expect(git.error).toBeUndefined();expect(validPushDestination(name),name).toBe(git.status===0&&!name.startsWith('-'));}
 expect(validPushDestination('a'.repeat(1025))).toBe(false);expect(validPushDestination('한'.repeat(342))).toBe(false);
});
