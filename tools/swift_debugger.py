"""Record, pack, and serve the local trading debugger (loopback only).

python tools/swift_debugger.py run --out runs/swift-debug --seed 7
python tools/swift_debugger.py serve --run runs/swift-debug
"""
import argparse
import functools
import hashlib
from http.server import BaseHTTPRequestHandler, HTTPServer
import json
import mimetypes
import os
from pathlib import Path
import subprocess
import sys
import time
from urllib.parse import unquote, urlparse
import webbrowser

from build_release import windows_build_budget
from debugger_data import build

ROOT=Path(__file__).resolve().parents[1]
ASSETS=ROOT/"tools/debugger"


class Handler(BaseHTTPRequestHandler):
    def __init__(self,*args,run_dir,**kwargs):
        self.run_dir=run_dir
        super().__init__(*args,**kwargs)

    def do_GET(self):
        url=unquote(urlparse(self.path).path)
        if url.startswith("/data/"):
            base=self.run_dir/"data";rel=url.removeprefix("/data/")
        else:
            base=ASSETS;rel="index.html" if url=="/" else url.lstrip("/")
        path=(base/rel).resolve()
        if not path.is_relative_to(base.resolve()) or not path.is_file():
            self.send_error(404);return
        raw=path.read_bytes()
        self.send_response(200)
        self.send_header("Content-Type","application/json; charset=utf-8" if path.name.endswith(".json.gz") else (mimetypes.guess_type(path.name)[0] or "application/octet-stream"))
        if path.name.endswith(".json.gz"):self.send_header("Content-Encoding","gzip")
        self.send_header("Content-Length",str(len(raw)))
        self.send_header("Cache-Control","no-cache")
        self.send_header("X-Content-Type-Options","nosniff")
        self.end_headers()
        try:self.wfile.write(raw)
        except (BrokenPipeError,ConnectionResetError):pass

    def log_message(self,*_):pass


def main():
    p=argparse.ArgumentParser(description=__doc__)
    sub=p.add_subparsers(dest="command",required=True)
    run=sub.add_parser("run")
    run.add_argument("--out",required=True)
    run.add_argument("--scenario",default="scenarios/baseline.toml")
    run.add_argument("--seed",type=int,default=7)
    run.add_argument("--threads",type=int,default=4)
    run.add_argument("--ticks",type=int)
    run.add_argument("--challenge")
    run.add_argument("--build",action="store_true",help="Rebuild the release recorder first")
    pack=sub.add_parser("pack");pack.add_argument("--run",required=True)
    serve=sub.add_parser("serve");serve.add_argument("--run",required=True)
    serve.add_argument("--port",type=int,default=8765);serve.add_argument("--open",action="store_true")
    args=p.parse_args()
    if args.command=="run":
        budget=windows_build_budget() if os.name=="nt" else (os.cpu_count() or 1)
        binary=ROOT/"target/release"/("debug-run.exe" if os.name=="nt" else "debug-run")
        sources=[ROOT/"Cargo.lock",*ROOT.glob("crates/**/*.rs"),*ROOT.glob("crates/**/Cargo.toml")]
        if args.build or not binary.exists() or any(p.stat().st_mtime>binary.stat().st_mtime for p in sources):
            env=os.environ.copy()
            # A machine-wide sccache socket may be excluded on Windows. Avoid
            # needing a cache daemon for the standalone debugger launcher.
            env["RUSTC_WRAPPER"]=""
            subprocess.run(["cargo","build","--release","--bin","debug-run","--jobs",str(budget)],cwd=ROOT,env=env,check=True)
        dest=(ROOT/args.out).resolve()
        if dest.exists():p.error(f"Output already exists: {dest}")
        command=[str(binary),str((ROOT/args.scenario).resolve()),"--out",str(dest),"--seed",str(args.seed),"--threads",str(args.threads)]
        if args.ticks is not None:command += ["--ticks",str(args.ticks)]
        if args.challenge:command += ["--challenge",args.challenge]
        started=time.perf_counter()
        subprocess.run([sys.executable,str(ROOT/"tools/arena_gate.py"),"--cwd",str(ROOT),"--",*command],cwd=ROOT,check=True)
        record_seconds=time.perf_counter()-started
        (dest/"source-scenario.toml").write_bytes((ROOT/args.scenario).read_bytes())
        started=time.perf_counter()
        manifest=build(dest)
        provenance=dict(command=command,cpu_budget=budget,source=manifest["source_trace_sha256"],
            binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
            swift_sha256=hashlib.sha256((ROOT/"crates/bots/src/swift.rs").read_bytes()).hexdigest(),
            record_including_gate_seconds=record_seconds,pack_seconds=time.perf_counter()-started,
            source_files_sha256={str(p.relative_to(ROOT)):hashlib.sha256(p.read_bytes()).hexdigest() for p in sources})
        (dest/"command.json").write_text(json.dumps(provenance,indent=2),encoding="utf-8")
        print(f"Ready: python tools/swift_debugger.py serve --run {args.out} --open")
    elif args.command=="pack":
        if os.name=="nt":windows_build_budget()
        build((ROOT/args.run).resolve())
    else:
        directory=(ROOT/args.run).resolve()
        if not (directory/"data/manifest.json").exists():p.error("No packed data; run the pack command first")
        server=HTTPServer(("127.0.0.1",args.port),functools.partial(Handler,run_dir=directory))
        url=f"http://127.0.0.1:{server.server_port}"
        print(f"Trading debugger: {url} · {directory}",flush=True)
        if args.open:webbrowser.open(url)
        try:server.serve_forever()
        except KeyboardInterrupt:pass
        finally:server.server_close()


if __name__=="__main__":main()
