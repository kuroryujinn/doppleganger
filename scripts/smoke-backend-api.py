#!/usr/bin/env python3
"""Upload a prepared ZIP, submit/poll a job and verify downloaded evidence."""
import argparse
import hashlib
import http.client
import ipaddress
import json
import os
from pathlib import Path
import time
import urllib.parse
import uuid


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", default="http://127.0.0.1:8787")
    parser.add_argument("--metadata", type=Path, required=True)
    parser.add_argument("--archive", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--upload-only", action="store_true")
    args = parser.parse_args()
    url = urllib.parse.urlsplit(args.url)
    if url.scheme not in ("http", "https") or not url.hostname or url.username or url.password or url.query or url.fragment or url.path not in ("", "/"):
        parser.error("use an HTTP(S) API origin")
    if url.scheme == "http":
        try:
            local = url.hostname == "localhost" or ipaddress.ip_address(url.hostname).is_loopback
        except ValueError:
            local = False
        if not local:
            parser.error("remote API connections require HTTPS")
    token = os.environ.get("DG_API_TOKEN", "")
    if len(token) < 32 or len(token) > 4096 or not token.isascii() or not all(33 <= ord(c) <= 126 for c in token):
        parser.error("set DG_API_TOKEN to an operator-provisioned bearer token")
    metadata = args.metadata.read_bytes()
    size = args.archive.stat().st_size
    if len(metadata) > 32 * 1024 or not 0 < size <= 384 * 1024 * 1024:
        parser.error("metadata/archive exceeds API limits")
    args.output.mkdir(mode=0o700, parents=True, exist_ok=False)

    def connect():
        cls = http.client.HTTPSConnection if url.scheme == "https" else http.client.HTTPConnection
        return cls(url.hostname, url.port, timeout=130)

    def response(connection):
        reply = connection.getresponse()
        if not 200 <= reply.status < 300:
            text = reply.read(32 * 1024).decode("utf-8", errors="replace")
            connection.close()
            raise RuntimeError(f"API returned HTTP {reply.status}: {text}")
        return reply

    def request(method, path, payload=None):
        connection = connect()
        headers = {"Authorization": "Bearer " + token}
        body = None
        if payload is not None:
            headers["Content-Type"] = "application/json"
            headers["Idempotency-Key"] = "smoke-" + uuid.uuid4().hex
            body = json.dumps(payload).encode()
        connection.request(method, path, body=body, headers=headers)
        reply = response(connection)
        data = reply.read(128 * 1024 * 1024 + 1)
        connection.close()
        if len(data) > 128 * 1024 * 1024:
            raise RuntimeError("JSON response exceeds output budget")
        return json.loads(data)

    def save(name, value):
        (args.output / name).write_text(json.dumps(value, indent=2) + "\n")

    save("capabilities.json", request("GET", "/v1/capabilities"))
    boundary = "dg-smoke-" + uuid.uuid4().hex
    begin = (f'--{boundary}\r\nContent-Disposition: form-data; name="metadata"\r\nContent-Type: application/json\r\n\r\n').encode() + metadata
    begin += (f'\r\n--{boundary}\r\nContent-Disposition: form-data; name="archive"; filename="project.zip"\r\nContent-Type: application/zip\r\n\r\n').encode()
    end = (f"\r\n--{boundary}--\r\n").encode()
    connection = connect()
    connection.putrequest("POST", "/v1/submissions")
    connection.putheader("Authorization", "Bearer " + token)
    connection.putheader("Content-Type", "multipart/form-data; boundary=" + boundary)
    connection.putheader("Content-Length", str(len(begin) + size + len(end)))
    connection.endheaders()
    connection.send(begin)
    uploaded_hash = hashlib.sha256()
    with args.archive.open("rb") as file:
        remaining = size
        while remaining:
            chunk = file.read(min(64 * 1024, remaining))
            if not chunk:
                raise RuntimeError("Archive changed while uploading")
            connection.send(chunk)
            uploaded_hash.update(chunk)
            remaining -= len(chunk)
    connection.send(end)
    reply = response(connection)
    submission = json.loads(reply.read(64 * 1024))
    connection.close()
    if submission["sha256"] != uploaded_hash.hexdigest() or submission["bytes"] != size:
        raise RuntimeError("Upload receipt does not match the streamed archive")
    save("submission.json", submission)
    if request("GET", "/v1/submissions/" + submission["submission_id"]) != submission:
        raise RuntimeError("Stored submission differs from receipt")
    if args.upload_only:
        print(json.dumps({"submission_id": submission["submission_id"], "uploaded": True, "execution_tested": False}))
        return
    job = request("POST", "/v1/jobs", {"submission_id": submission["submission_id"]})
    path = "/v1/jobs/" + job["job_id"]
    save("job.json", job)
    deadline = time.monotonic() + 600
    while job["state"] not in ("completed", "failed", "cancelled"):
        if time.monotonic() >= deadline:
            raise RuntimeError("Polling deadline exceeded; job ID is saved in job.json")
        time.sleep(2)
        job = request("GET", path)
        save("job.json", job)
    save("events.json", request("GET", path + "/events"))
    if job["state"] != "completed":
        raise RuntimeError(f"Job ended as {job['state']}: {job.get('error_code')}")
    save("report.json", request("GET", path + "/report"))
    target = args.output / "artifacts"
    target.mkdir(mode=0o700)
    names = {"report_json": "report.json", "report_html": "report.html", "summary": "summary.md", "portable_case": "portable_case.zip"}
    for artifact in job["artifacts"]:
        name = names[artifact["kind"]]
        connection = connect()
        connection.request("GET", path + "/artifacts/" + artifact["kind"], headers={"Authorization": "Bearer " + token})
        reply = response(connection)
        sha = hashlib.sha256()
        length = 0
        with (target / name).open("xb") as file:
            while chunk := reply.read(64 * 1024):
                length += len(chunk)
                if length > 128 * 1024 * 1024:
                    raise RuntimeError("Artifact exceeds output budget")
                sha.update(chunk)
                file.write(chunk)
        connection.close()
        if length != artifact["bytes"] or sha.hexdigest() != artifact["sha256"]:
            raise RuntimeError("Downloaded artifact does not match inventory")
    print(json.dumps({"job_id": job["job_id"], "state": job["state"], "verdict": job["verdict"], "artifacts_verified": len(job["artifacts"])}))


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError, RuntimeError, http.client.HTTPException) as error:
        raise SystemExit(str(error)) from None
