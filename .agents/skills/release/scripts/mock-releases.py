"""A stand-in for Mothr's GitHub releases, to test install.sh and the in-app updaters offline.

    python3 -I mock-releases.py <root> <port>

<root>/<version>/ holds a release's files (tarball, APKs, SHA256SUMS); <root>/latest holds the
newest version. It answers:
  GET /releases/latest              302 to /releases/tag/v<latest>   (install.sh: MOTHR_RELEASES_URL=http://host:port/releases)
  GET /releases/download/v<v>/<f>   the file
  GET /api/releases/latest          GitHub-like JSON                  (desktop: MOTHR_UPDATE_API=http://host:port/api)
Set PUBLIC_BASE to the address clients use for downloads (e.g. http://10.0.2.2:<port> for the
Android emulator).
"""
import http.server, json, os, sys
ROOT, PORT, HOST = sys.argv[1], int(sys.argv[2]), (sys.argv[3] if len(sys.argv) > 3 else "127.0.0.1")
PUBLIC = os.environ.get("PUBLIC_BASE", f"http://{HOST}:{PORT}")
class H(http.server.BaseHTTPRequestHandler):
    def latest(self): return open(os.path.join(ROOT, "latest")).read().strip()
    def do_HEAD(self): self.do_GET(head=True)
    def do_GET(self, head=False):
        p = self.path.split("?")[0]
        if p == "/releases/latest":
            self.send_response(302); self.send_header("Location", f"/releases/tag/v{self.latest()}"); self.end_headers(); return
        if p.startswith("/releases/tag/"):
            self.send_response(200); self.send_header("Content-Type", "text/html"); self.end_headers()
            if not head: self.wfile.write(b"<html>release</html>")
            return
        if p.startswith("/releases/download/v"):
            v, name = p[len("/releases/download/v"):].split("/", 1)
            f = os.path.join(ROOT, v, name)
            if not os.path.isfile(f): self.send_error(404); return
            self.send_response(200); self.send_header("Content-Length", str(os.path.getsize(f))); self.end_headers()
            if not head:
                with open(f, "rb") as fh: self.wfile.write(fh.read())
            return
        if p == "/api/releases/latest":
            v = self.latest(); d = os.path.join(ROOT, v)
            assets = [{"name": n, "browser_download_url": f"{PUBLIC}/releases/download/v{v}/{n}", "size": os.path.getsize(os.path.join(d, n))} for n in sorted(os.listdir(d))]
            body = json.dumps({"tag_name": f"v{v}", "html_url": f"{PUBLIC}/releases/tag/v{v}", "draft": False, "prerelease": False, "assets": assets}).encode()
            self.send_response(200); self.send_header("Content-Type", "application/json"); self.send_header("Content-Length", str(len(body))); self.end_headers()
            if not head: self.wfile.write(body)
            return
        self.send_error(404)
    def log_message(self, *a): pass
http.server.ThreadingHTTPServer(("0.0.0.0", PORT), H).serve_forever()
