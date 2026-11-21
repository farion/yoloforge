#!/usr/bin/env python3
"""Real Factory/CLI/two-runner checkpoint transport; no OpenCode or deployment."""
import json
import argparse
import os
from pathlib import Path
import secrets
import socket
import subprocess
import tempfile
import time
import urllib.request
import uuid

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--review",action="store_true",help="Also test Docker-contained review and CAS integration")
    parser.add_argument("--review-image",default="yoloforge-freezer-test:development")
    options=parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="handoff-", dir="/tmp/opencode") as tmp:
        base = Path(tmp)
        env = {k: v for k, v in os.environ.items() if k != "FACTORY_TOKEN_FILE"}
        env["FACTORY_TOKEN"] = secrets.token_hex(32)
        processes, logs = [], []
        reviews=[]
        def spawn(binary, *args):
            log = open(base / f"process-{len(logs)}.log", "w+")
            logs.append(log)
            p = subprocess.Popen([str(ROOT / "target/debug" / binary), *map(str, args)],
                                 env=env, stdout=log, stderr=log)
            processes.append(p)
            return p
        def stop(p):
            if p.poll() is None:
                p.terminate()
                try:
                    p.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    p.kill()
                    p.wait()
        def wait(fn):
            for _ in range(200):
                try:
                    value = fn()
                    if value:
                        return value
                except (OSError, ValueError):
                    pass
                time.sleep(.1)
            raise RuntimeError("Timeout waiting for handoff state")
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            address = f"127.0.0.1:{sock.getsockname()[1]}"
        url = "http://" + address
        def snapshot():
            request = urllib.request.Request(url + "/v1/snapshot", headers={
                "Authorization": "Bearer " + env["FACTORY_TOKEN"], "x-factory-protocol": "1"})
            with urllib.request.build_opener(urllib.request.ProxyHandler({})).open(request, timeout=2) as r:
                return json.load(r)
        def cli(*args):
            result = subprocess.run([str(ROOT / "target/debug/yoloforge"), "--server", url, *map(str, args)],
                                    env=env, capture_output=True, text=True, timeout=15)
            if result.returncode:
                raise RuntimeError(result.stderr)
            return json.loads(result.stdout)
        def git(path, *args):
            return subprocess.check_output(["git", "-C", str(path), *args], stderr=subprocess.DEVNULL).decode().strip()
        def runner(which):
            return spawn("factory-runner", "--server", url, "--id", ids[which], "--name", "handoff-" + which,
                         "--journal", base / f"{which}.sqlite3", "--repositories", base / f"{which}.json",
                         "--workspace-root", base / f"work-{which}", *(["--review-image",options.review_image] if options.review else []))
        def stream():
            return next(s for s in snapshot()["streams"] if s["id"] == stream_id)
        def command(action):
            c = cli("stream", action, stream_id, "--expected-revision", stream()["revision"])
            def completed():
                return next((x for x in snapshot()["commands"] if x["id"] == c["id"] and x["state"] in ("succeeded", "failed")), None)
            result = wait(completed)
            assert result["state"] == "succeeded", result
            return result
        def finished(command_id):
            return next((x for x in snapshot()["commands"] if x["id"]==command_id and x["state"] in ("succeeded","failed")),None)
        try:
            server = spawn("factoryd", "--listen", address, "--database", base / "factory.sqlite3")
            wait(snapshot)
            project = cli("project", "create", "--name", "Handoff", "--repository", "https://example.invalid/project.git")
            ids = {name: str(uuid.uuid4()) for name in ("a", "b")}
            for name in ids:
                repo = base / ("repo-" + name)
                repo.mkdir()
                git(repo, "init", "-b", "main")
                git(repo, "config", "user.name", "Smoke")
                git(repo, "config", "user.email", "smoke@example.invalid")
                (repo / "file").write_text("base-" + name)
                git(repo, "add", ".")
                git(repo, "commit", "-m", "initial")
                (base / f"{name}.json").write_text(json.dumps({project["id"]: str(repo)}))
            a, b = runner("a"), runner("b")
            wait(lambda: len(snapshot()["runners"]) == 2)
            created = cli("stream", "create", "--project", project["id"], "--runner", ids["a"], "--goal", "Transfer dirty work")
            stream_id = created["id"]
            prepared = command("prepare")
            original = Path(prepared["result"]["workspace"]["path"])
            (original / "file").write_text("dirty source A")
            (original / "new-ü").write_bytes(b"\x00\xffbinary")
            before = git(original, "status", "--porcelain")
            first = command("checkpoint")
            cp = first["result"]["checkpoint"]
            assert git(original, "status", "--porcelain") == before
            # Drop every process after upload; transfer must use persistent Factory bytes.
            for p in (a, b, server):
                stop(p)
            server = spawn("factoryd", "--listen", address, "--database", base / "factory.sqlite3")
            wait(snapshot)
            handoff_id = str(uuid.uuid4())
            args = ("stream", "handoff", stream_id, "--expected-revision", stream()["revision"],
                    "--checkpoint", first["id"], "--runner", ids["b"], "--reason", "Move to B", "--request-id", handoff_id)
            moved = cli(*args)
            assert cli(*args) == moved and moved["ownership_epoch"] == 2
            # Source artifact is unavailable: B must download from Factory.
            Path(cp["bundle_path"]).unlink()
            a, b = runner("a"), runner("b")
            wait(lambda: len(snapshot()["runners"]) == 2)
            target = command("prepare")
            destination = Path(target["result"]["workspace"]["path"])
            assert git(destination, "rev-parse", "HEAD") == cp["commit"]
            assert (destination / "file").read_text() == "dirty source A"
            assert (destination / "new-ü").read_bytes() == b"\x00\xffbinary"
            assert git(destination, "status", "--porcelain") == ""
            print("PASS checkpoint upload survives Factory/runner restart; independent target downloads pinned commit")
            (destination / "file").write_text("change on B")
            second = command("checkpoint")
            cli("stream", "handoff", stream_id, "--expected-revision", stream()["revision"],
                "--checkpoint", second["id"], "--runner", ids["a"], "--reason", "Return to A")
            returned = command("prepare")
            final = Path(returned["result"]["workspace"]["path"])
            assert stream()["ownership_epoch"] == 3 and final != original
            assert (final / "file").read_text() == "change on B"
            assert (original / "file").read_text() == "dirty source A"
            assert git(original, "status", "--porcelain") == before
            print("PASS reverse handoff creates epoch-3 worktree; original source files/index remain intact")
            if options.review:
                repo=base/"repo-a"
                git(repo,"checkout","--detach")
                def configure(test):
                    p=next(p for p in snapshot()["projects"] if p["id"]==project["id"])
                    cli("project","configure",p["id"],"--expected-revision",p["revision"],"--provider","openai","--model","test","--test-command",test)
                def review():
                    cp=command("checkpoint")
                    c=cli("stream","review",stream_id,"--expected-revision",stream()["revision"],"--checkpoint",cp["id"],"--expected-target-commit",git(repo,"rev-parse","main"))
                    reviews.append(c["id"])
                    return c
                configure("python3 -c \"from pathlib import Path; import time; assert Path('file').read_text()=='change on B'; print('review-once', flush=True); time.sleep(12)\"")
                reviewed=review()
                def running_test():
                    ids=subprocess.check_output(["docker","ps","-q","--filter",f"label=io.yoloforge.review={reviewed['id']}"],text=True).strip()
                    return ids
                wait(running_test)
                time.sleep(.5)  # allow the runner to persist observed start identity
                stop(a);stop(server)
                server=spawn("factoryd","--listen",address,"--database",base/"factory.sqlite3")
                wait(snapshot);a=runner("a")
                tested=wait(lambda:finished(reviewed["id"]))
                assert tested["state"]=="succeeded",tested
                evidence=tested["result"]["review"]
                assert Path(evidence["log_path"]).read_text().count("review-once")==1
                integrated=cli("stream","integrate",stream_id,"--expected-revision",stream()["revision"],"--review",reviewed["id"])
                applied=wait(lambda:finished(integrated["id"]))
                assert applied["state"]=="succeeded",applied
                assert git(repo,"rev-parse","main")==evidence["candidate_commit"]
                print("PASS isolated review tests survive restart exactly once; tested candidate integrated by CAS")
                configure("exit 9")
                target_before=git(repo,"rev-parse","main")
                rejected=review();failed=wait(lambda:finished(rejected["id"]))
                assert failed["state"]=="failed",failed
                assert git(repo,"rev-parse","main")==target_before
                configure("python3 -c \"print('ok')\"")
                valid=review();assert wait(lambda:finished(valid["id"]))["state"]=="succeeded"
                changed=git(repo,"commit-tree",git(repo,"rev-parse","main^{tree}"),"-p",target_before,"-m","concurrent target change")
                git(repo,"update-ref","refs/heads/main",changed,target_before)
                attempted=cli("stream","integrate",stream_id,"--expected-revision",stream()["revision"],"--review",valid["id"])
                assert wait(lambda:finished(attempted["id"]))["state"]=="failed"
                assert git(repo,"rev-parse","main")==changed
                print("PASS failed tests and concurrent target changes prevent integration without overwriting work")
        except Exception:
            for log in logs:
                log.flush()
                log.seek(0)
                print(log.read()[-4000:])
            raise
        finally:
            for p in reversed(processes):
                stop(p)
            for log in logs:
                log.close()
            for review_id in reviews:
                ids=subprocess.run(["docker","ps","-aq","--no-trunc","--filter",f"label=io.yoloforge.review={review_id}"],capture_output=True,text=True).stdout.split()
                for runtime in ids:
                    subprocess.run(["docker","rm","-f",runtime],capture_output=True)


if __name__ == "__main__":
    main()
