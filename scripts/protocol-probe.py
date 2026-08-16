#!/usr/bin/env python3
"""Independent Python probe for Outboard's framed worker protocol."""

import json
import struct
import subprocess
import sys
from pathlib import Path

MAX = 16 * 1024 * 1024


def spawn(path):
    return subprocess.Popen(
        [str(path), "__outboard", "serve"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )


def send(process, value):
    body = json.dumps(value, separators=(",", ":")).encode()
    process.stdin.write(struct.pack(">I", len(body)) + body)
    process.stdin.flush()


def recv(process):
    header = process.stdout.read(4)
    assert len(header) == 4, f"short header {header!r}"
    length = struct.unpack(">I", header)[0]
    body = process.stdout.read(length)
    assert len(body) == length, f"short body {len(body)}/{length}"
    return json.loads(body)


def hello():
    return {
        "type": "hello",
        "hello": {
            "framework": "0.1.0",
            "protocol": "1.0.0",
            "requested_interfaces": [{"id": "demo.echo", "version": "^1.0"}],
            "metadata": {},
        },
    }


def utf8(value):
    return {"encoding": "utf8", "data": value}


def main():
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(sys.argv[0]).name} PLUGIN")
    path = Path(sys.argv[1]).resolve()
    assert path.is_file(), path

    process = spawn(path)
    send(process, hello())
    response = recv(process)
    assert response["type"] == "hello", response
    assert response["hello"]["manifest"]["plugin"]["id"] == {
        "namespace": "outboard-demo",
        "kind": "engine",
        "name": "echo",
    }, response

    send(process, {"type": "ping", "nonce": 123456})
    assert recv(process) == {"type": "pong", "nonce": 123456}

    send(
        process,
        {
            "type": "invoke",
            "request": {
                "id": 77,
                "interface": "demo.echo",
                "command": "echo",
                "args": [
                    utf8("Python"),
                    utf8("--repeat"),
                    utf8("2"),
                    utf8("--case"),
                    utf8("upper"),
                    utf8("--tag"),
                    utf8("probe"),
                ],
                "metadata": {},
            },
        },
    )
    frames = []
    while True:
        frame = recv(process)
        frames.append(frame)
        if frame["type"] in ("finished", "error") and frame.get("id") == 77:
            break
    kinds = [frame["type"] for frame in frames]
    assert kinds[0] == "started", kinds
    assert "progress" in kinds, kinds
    assert "output" in kinds, kinds
    assert kinds[-1] == "finished", frames[-1]
    result = frames[-1]["result"]["result"]
    assert result["kind"] == "json", result
    assert result["value"]["text"] == "PYTHON PYTHON", result
    assert result["value"]["tags"] == ["probe"], result
    output = next(frame for frame in frames if frame["type"] == "output")["payload"]
    assert output["kind"] == "json", output
    assert output["value"]["process_id"] == result["value"]["process_id"], frames
    print("PASS independent invoke/progress/output/result")

    send(process, hello())
    response = recv(process)
    assert response["type"] == "error" and response["error"]["code"] == "duplicate_hello", response

    send(process, {"type": "shutdown"})
    assert recv(process)["type"] == "shutdown_ack"
    process.stdin.close()
    assert process.wait(timeout=5) == 0
    print("PASS handshake/ping/duplicate-hello/shutdown")

    process = spawn(path)
    process.stdin.write(struct.pack(">I", 1) + b"{")
    process.stdin.flush()
    process.stdin.close()
    assert process.wait(timeout=5) != 0
    print("PASS malformed JSON rejected")

    process = spawn(path)
    process.stdin.write(struct.pack(">I", MAX + 1))
    process.stdin.flush()
    process.stdin.close()
    assert process.wait(timeout=5) != 0
    print("PASS oversized frame rejected")

    print("protocol probe passed")


if __name__ == "__main__":
    main()
