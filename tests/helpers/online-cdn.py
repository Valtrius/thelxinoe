"""Loopback provider transport for real FFmpeg playback tests; no upstream network."""
import functools
import http.server
import json
import pathlib
import select
import socket
import socketserver
import ssl
import subprocess
import sys
import threading

root = pathlib.Path(sys.argv[1])
subprocess.run(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", str(root / "key.pem"), "-out", str(root / "cert.pem"), "-days", "1", "-subj", "/CN=fixture.googlevideo.com"], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
class MediaHandler(http.server.SimpleHTTPRequestHandler):
    def do_GET(self):
        if self.path.startswith('/denied.mp4'):
            self.send_error(403)
            return
        super().do_GET()


server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(MediaHandler, directory=str(root)))
tls = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
tls.load_cert_chain(root / "cert.pem", root / "key.pem")
server.socket = tls.wrap_socket(server.socket, server_side=True)
threading.Thread(target=server.serve_forever, daemon=True).start()


class Tunnel(socketserver.BaseRequestHandler):
    def handle(self):
        headers = b""
        while b"\r\n\r\n" not in headers:
            data = self.request.recv(4096)
            if not data or len(headers) > 8192:
                return
            headers += data
        if not headers.startswith(b"CONNECT fixture.googlevideo.com:443 "):
            return
        with socket.create_connection(server.server_address) as upstream:
            self.request.sendall(b"HTTP/1.1 200 Connection established\r\n\r\n")
            while True:
                ready, _, _ = select.select([self.request, upstream], [], [], 20)
                if not ready:
                    return
                for source in ready:
                    data = source.recv(65536)
                    if not data:
                        return
                    (upstream if source is self.request else self.request).sendall(data)


proxy = socketserver.ThreadingTCPServer(("127.0.0.1", 0), Tunnel)
proxy.daemon_threads = True
print(json.dumps({"proxy": f"http://127.0.0.1:{proxy.server_address[1]}"}), flush=True)
proxy.serve_forever()
