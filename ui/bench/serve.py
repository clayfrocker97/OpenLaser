"""Loopback-only static benchmark UI, API proxy and result collector."""
import argparse
import datetime
import http.client
import http.server
import json
import mimetypes
from pathlib import Path
import urllib.parse
import uuid


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--work", type=Path, required=True)
    parser.add_argument("--backend-port", type=int, default=8111)
    parser.add_argument("--port", type=int, default=8280)
    args = parser.parse_args()
    root = args.root.resolve()
    (args.work / "results").mkdir(parents=True, exist_ok=True)

    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def reply(self, value, status=200):
            body = json.dumps(value).encode()
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Cache-Control", "no-store")
            self.end_headers()
            self.wfile.write(body)

        def proxy(self):
            connection = http.client.HTTPConnection("127.0.0.1", args.backend_port, timeout=240)
            body = self.rfile.read(int(self.headers.get("Content-Length", "0")))
            connection.request(self.command, self.path, body=body or None,
                               headers={"Content-Type": self.headers.get("Content-Type", "application/json"),
                                        **{key: self.headers[key] for key in ["Host", "Origin", "Sec-Fetch-Site", "X-OpenLaser-Client", "X-OpenLaser-Name", "X-OpenLaser-Previous"] if key in self.headers}})
            response = connection.getresponse()
            self.send_response(response.status)
            for name in ["Content-Type", "Content-Length", "Cache-Control", "X-Frame-Options", "Content-Security-Policy"]:
                if response.getheader(name):
                    self.send_header(name, response.getheader(name))
            self.end_headers()
            try:
                if self.path.startswith("/api/events"):
                    while line := response.readline():
                        self.wfile.write(line)
                        self.wfile.flush()
                else:
                    while chunk := response.read(65536):
                        self.wfile.write(chunk)
            except (BrokenPipeError, ConnectionResetError):
                pass
            finally:
                connection.close()

        def do_GET(self):
            if self.path.startswith("/api/"):
                return self.proxy()
            if self.path == "/bench/manifest":
                return self.reply(json.loads((args.work / "fixtures/manifest.json").read_text()))
            url = urllib.parse.urlparse(self.path).path
            path = (root / urllib.parse.unquote(url).lstrip("/")).resolve()
            if root not in path.parents and path != root:
                return self.reply({"error": "outside static root"}, 403)
            if path.is_dir():
                path = path / "index.html"
            if url.rstrip("/") in ["/app", "/mobile"]:
                path = root / "index.html"
            if not path.is_file():
                return self.reply({"error": "not found"}, 404)
            body = path.read_bytes()
            self.send_response(200)
            self.send_header("Content-Type", mimetypes.guess_type(path)[0] or "application/octet-stream")
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Cache-Control", "no-store")
            self.end_headers()
            self.wfile.write(body)

        def do_POST(self):
            if self.path.startswith("/api/"):
                return self.proxy()
            if self.path != "/bench/result":
                return self.reply({"error": "not found"}, 404)
            length = int(self.headers.get("Content-Length", "0"))
            if length < 1 or length > 8_000_000:
                return self.reply({"error": "invalid result size"}, 413)
            result = json.loads(self.rfile.read(length))
            result["collected_utc"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
            identity = uuid.uuid4().hex
            (args.work / "results" / f"{identity}.json").write_text(json.dumps(result, indent=2) + "\n")
            self.reply({"id": identity})

        do_DELETE = proxy

    server = http.server.ThreadingHTTPServer(("127.0.0.1", args.port), Handler)
    print(f"Benchmark UI: http://127.0.0.1:{args.port}/baseline/", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
