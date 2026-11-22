"""Isolated Git CLI oracle for remote-removal implementation semantics."""
import hashlib,json,os,pathlib,subprocess,tempfile
ROOT=pathlib.Path(__file__).resolve().parents[2]
env=dict(os.environ,GIT_CONFIG_NOSYSTEM='1',GIT_CONFIG_GLOBAL=os.devnull,GIT_OPTIONAL_LOCKS='0',GIT_AUTHOR_NAME='Oracle',GIT_AUTHOR_EMAIL='oracle@example.invalid',GIT_COMMITTER_NAME='Oracle',GIT_COMMITTER_EMAIL='oracle@example.invalid')
results=[]
for scenario in ['standard','packed','shared-destination','custom-destination','local-destination','negative-own','negative-shared','source-only','empty-destination']:
 with tempfile.TemporaryDirectory(prefix='branchdesk-remote-remove-oracle-') as temp:
  root=pathlib.Path(temp)
  def git(*args,input=None,check=True):
   p=subprocess.run(['git','-C',temp,*args],input=input,env=env,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
   if check and p.returncode:raise RuntimeError(p.stderr.decode())
   return p
  git('init','-b','main');(root/'file.txt').write_text('base\n');git('add','file.txt');git('commit','-m','base')
  oid=git('rev-parse','HEAD').stdout.decode().strip();git('remote','add','origin','https://example.invalid/origin.git')
  git('remote','add','keep','https://example.invalid/keep.git')
  git('update-ref','refs/remotes/origin/main',oid);git('symbolic-ref','refs/remotes/origin/HEAD','refs/remotes/origin/main');git('update-ref','refs/remotes/keep/main',oid)
  git('config','branch.main.remote','origin');git('config','branch.main.merge','refs/heads/main');git('config','branch.main.pushRemote','origin');git('config','branch.main.description','preserve');git('config','remote.pushDefault','origin')
  if scenario=='packed':git('pack-refs','--all','--prune')
  if scenario=='shared-destination':git('config','--add','remote.keep.fetch','+refs/heads/*:refs/remotes/origin/*')
  if scenario=='custom-destination':
   git('config','remote.origin.fetch','+refs/heads/*:refs/remotes/custom/*');git('update-ref','refs/remotes/custom/main',oid)
  if scenario=='local-destination':
   git('config','remote.origin.fetch','+refs/heads/*:refs/heads/import/*');git('update-ref','refs/heads/import/main',oid)
  if scenario=='negative-own':git('config','--add','remote.origin.fetch','^refs/heads/main')
  if scenario=='negative-shared':
   git('config','--add','remote.keep.fetch','+refs/heads/*:refs/remotes/origin/*');git('config','--add','remote.keep.fetch','^refs/heads/main')
  if scenario=='source-only':git('config','remote.origin.fetch','refs/heads/main')
  if scenario=='empty-destination':git('config','remote.origin.fetch','refs/heads/main:')
  def refs():return sorted(git('for-each-ref','--format=%(refname)').stdout.decode().splitlines())
  def config():return git('config','--local','--list').stdout.decode().splitlines()
  before=refs();before_config=config();index=(root/'.git/index').read_bytes();head=(root/'.git/HEAD').read_bytes();content=(root/'file.txt').read_bytes()
  removed=git('remote','remove','origin',check=False)
  after=refs();after_config=config()
  assert removed.returncode==0
  assert (root/'.git/index').read_bytes()==index and (root/'.git/HEAD').read_bytes()==head and (root/'file.txt').read_bytes()==content
  assert 'refs/remotes/keep/main' in after
  results.append(dict(scenario=scenario,removedRefs=sorted(set(before)-set(after)),preservedRefs=after,symbolicHeadFile=(root/'.git/refs/remotes/origin/HEAD').read_text() if (root/'.git/refs/remotes/origin/HEAD').exists() else None,removedConfig=sorted(set(before_config)-set(after_config)),remainingTracking=[s for s in after_config if s.startswith(('branch.','remote.pushdefault'))],stderr=removed.stderr.decode(),headIndexWorktreePreserved=True))
report={'gitVersion':subprocess.check_output(['git','--version'],env=env).decode().strip(),'cases':results,'purpose':'Behavioral oracle only; product remote deletion is not implemented by this script.'}
(ROOT/'reports/remote-remove-extended-oracle.json').write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps(report,indent=2))
