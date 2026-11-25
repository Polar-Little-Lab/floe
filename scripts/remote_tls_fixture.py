"""Synthetic smart-HTTP fixture. Used only by Rust network-worker integration tests."""
import http.server
import json
from pathlib import Path
import ssl
import subprocess
import sys
import time
import os
git_env={'PATH':'/usr/bin:/bin','GIT_CONFIG_NOSYSTEM':'1','GIT_CONFIG_GLOBAL':'/dev/null','GIT_OPTIONAL_LOCKS':'0'}

root = Path(sys.argv[1])
mode = sys.argv[2]
(root/'cert.cnf').write_text('[req]\ndistinguished_name=dn\nx509_extensions=ext\n[dn]\n[ext]\nsubjectAltName=IP:127.0.0.1,DNS:localhost\nbasicConstraints=critical,CA:TRUE\nextendedKeyUsage=serverAuth\nkeyUsage=digitalSignature,keyEncipherment,keyCertSign\n')
subprocess.run(['/usr/bin/openssl', 'req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-keyout', str(root/'key.pem'), '-out', str(root/'cert.pem'), '-days', '1', '-subj', '/CN=localhost', '-config', str(root/'cert.cnf')], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
subprocess.run(['/usr/bin/openssl', 'x509', '-in', str(root/'cert.pem'), '-outform', 'DER', '-out', str(root/'cert.der')], check=True)
def pkt(text):
    data=text.encode()
    return f'{len(data)+4:04x}'.encode()+data
class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass
    def auth_allowed(self):
        if mode.endswith('-authenticated') and self.headers.get('Authorization') != 'Basic c3ludGhldGljOnRva2Vu':
            self.send_response(401)
            self.send_header('WWW-Authenticate','Basic realm="synthetic"')
            self.end_headers()
            return False
        return True
    def do_GET(self):
        with (root/'requests.jsonl').open('a') as log:
            log.write(json.dumps({'path':self.path,'authorization':self.headers.get('Authorization')})+'\n')
        if not self.auth_allowed(): return
        if mode.startswith('pack') or (mode.startswith('roundtrip') and 'git-upload-pack' in self.path):
            body=pkt('# service=git-upload-pack\n')+b'0000'+subprocess.check_output(['/usr/bin/git','upload-pack','--stateless-rpc','--advertise-refs',str(root/'source')],env=git_env)
            self.send_response(200)
            self.send_header('Content-Type','application/x-git-upload-pack-advertisement')
            self.send_header('Content-Length',str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        elif mode.startswith('push') or mode.startswith('roundtrip'):
            body=pkt('# service=git-receive-pack\n')+b'0000'+subprocess.check_output(['/usr/bin/git','receive-pack','--stateless-rpc','--advertise-refs',str(root/'source')],env=git_env)
            self.send_response(200)
            self.send_header('Content-Type','application/x-git-receive-pack-advertisement')
            self.send_header('Content-Length',str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        elif mode=='receive':
            body=pkt('# service=git-receive-pack\n')+b'0000'+pkt('1'*40+' refs/heads/main\0report-status delete-refs\n')+b'0000'
            self.send_response(200)
            self.send_header('Content-Type','application/x-git-receive-pack-advertisement')
            self.send_header('Content-Length',str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        elif mode.startswith('redirect'):
            self.send_response(302)
            host='localhost' if mode=='redirect-host' else '127.0.0.1'
            scheme='http' if mode=='redirect-http' else 'https'
            port=1 if mode=='redirect-port' else self.server.server_port
            self.send_header('Location',f'{scheme}://{host}:{port}/redirected/info/refs?service=git-upload-pack')
            self.end_headers()
        elif mode=='slow':
            time.sleep(2)
        elif mode=='oversized':
            self.send_response(200)
            self.send_header('Content-Type','application/x-git-upload-pack-advertisement')
            self.send_header('Content-Length',str(4*1024*1024+1))
            self.end_headers()
        elif mode=='auth':
            self.send_response(401)
            self.send_header('WWW-Authenticate','Basic realm="fixture"')
            self.end_headers()
        else:
            oid='1'*40
            body=pkt('# service=git-upload-pack\n')+b'0000'+pkt(oid+' HEAD\0multi_ack symref=HEAD:refs/heads/main\n')+pkt(oid+' refs/heads/main\n')+b'0000'
            self.send_response(200)
            self.send_header('Content-Type','application/x-git-upload-pack-advertisement')
            self.send_header('Content-Length',str(len(body)))
            self.end_headers()
            self.wfile.write(body)
    def do_POST(self):
        with (root/'requests.jsonl').open('a') as log:
            log.write(json.dumps({'path':self.path,'method':'POST','authorization':self.headers.get('Authorization')})+'\n')
        if not self.auth_allowed(): return
        length=int(self.headers.get('Content-Length','0'))
        if length>512*1024:
            self.send_error(413)
            return
        request=self.rfile.read(length)
        if mode.startswith('push') or (mode.startswith('roundtrip') and self.path.endswith('/git-receive-pack')):
            if mode=='push-redirect':
                self.send_response(302)
                self.send_header('Location',f'https://127.0.0.1:{self.server.server_port}/other/git-receive-pack')
                self.end_headers()
                return
            result=subprocess.run(['/usr/bin/git','receive-pack','--stateless-rpc',str(root/'source')],input=request,env=git_env,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
            body=result.stdout
            if mode=='push-drop':
                self.connection.close()
                return
            if mode=='push-truncate': body=body[:-4]
            self.send_response(200)
            self.send_header('Content-Type','text/plain' if mode=='push-badtype' else 'application/x-git-receive-pack-result')
            self.send_header('Content-Length',str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        if mode=='pack-redirect':
            self.send_response(302)
            self.send_header('Location',f'https://127.0.0.1:{self.server.server_port}/other/git-upload-pack')
            self.end_headers()
            return
        body=subprocess.check_output(['/usr/bin/git','upload-pack','--stateless-rpc',str(root/'source')],input=request,env=git_env,stderr=subprocess.DEVNULL)
        if mode=='pack-corrupt': body=body[:-1]+bytes([body[-1]^1])
        if mode=='pack-truncate': body=body[:-10]
        self.send_response(200)
        self.send_header('Content-Type','application/x-git-upload-pack-result')
        self.send_header('Content-Length',str(len(body)))
        self.end_headers()
        self.wfile.write(body)
server=http.server.HTTPServer(('127.0.0.1',0),Handler)
context=ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
context.load_cert_chain(root/'cert.pem',root/'key.pem')
server.socket=context.wrap_socket(server.socket,server_side=True)
(root/'port').write_text(str(server.server_port))
server.serve_forever()
