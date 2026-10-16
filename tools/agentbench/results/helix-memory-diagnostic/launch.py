import hashlib,json,os,pathlib,signal,subprocess,time,http.client,socket
root=pathlib.Path.cwd()
out=root/'target/enginebench-probes/http-memory-bm25-off'
out.mkdir(exist_ok=False)
manifest=json.loads((root/'target/agentbench-setup/installation.json').read_text())
binary=root/'target/enginebench-probes/helix-container-bm25-off'
manifest['binary_sha256']=hashlib.sha256(binary.read_bytes()).hexdigest()
source=root/'target/enginebench-probes/large-guarded/probe/data/fast-helix'
(out/'data').mkdir()
subprocess.run(['cp','-a','--reflink=auto',str(source),str(out/'data/user')],check=True)
with socket.socket() as s: s.bind(('127.0.0.1',16971))
env=os.environ|{'HELIX_DATA_DIR':str(out/'data'),'HELIX_PORT':'16971','RUST_LOG':'error'}
(out/'metadata.json').write_text(json.dumps({'version':manifest['version'],'commit':manifest['commit'],'server_sha256':manifest['binary_sha256'],'population':1001000,'bm25':False,'configuration_change':'Only generated config bm25 Some(true) -> Some(false); engine and generated query bodies unmodified','source_database':str(source),'argv':['taskset','-c','1-4',str(binary)],'environment':{k:env[k] for k in ('HELIX_DATA_DIR','HELIX_PORT','RUST_LOG')}},indent=2)+'\n')
with (out/'server.log').open('w') as log:
 p=subprocess.Popen(['taskset','-c','1-4',str(binary)],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
 try:
  for i in range(1800):
   if p.poll() is not None: raise RuntimeError('server exited')
   try:
    c=http.client.HTTPConnection('127.0.0.1',16971,timeout=1)
    c.request('POST','/Noop','{}',{'Content-Type':'application/json'})
    r=c.getresponse(); body=r.read(); c.close()
    if r.status==200 and b'agentbench-noop' in body: break
   except OSError: pass
   time.sleep(.1)
  else: raise RuntimeError('readiness timeout')
  subprocess.run(['taskset','-c','12-21','python3','tools/agentbench/helix_memory_repro.py','--server-pid',str(p.pid),'--population','1001000','--output',str(out/'client')],check=True)
 finally:
  if p.poll() is None:
   os.killpg(p.pid,signal.SIGTERM)
   try: p.wait(timeout=5)
   except subprocess.TimeoutExpired: os.killpg(p.pid,signal.SIGKILL);p.wait()
