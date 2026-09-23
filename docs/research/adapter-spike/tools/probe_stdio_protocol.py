#!/usr/bin/env python3
"""Bounded, no-login stdio protocol probes for public harness binaries.

The probe writes only inside caller-provided isolated home/workspace paths. It
does not submit a model prompt, authenticate, or copy credentials.
"""

from __future__ import annotations

import argparse
import json
import os
import queue
import subprocess
import threading
import time
from pathlib import Path
from typing import Any


REDACT_KEYS = {
    "access_token",
    "accessToken",
    "refresh_token",
    "refreshToken",
    "api_key",
    "apiKey",
    "token",
    "email",
    "team_id",
    "team_name",
    "team_role",
    "subscription_tier",
    "planType",
    "rateLimits",
    "credits",
    "hostname",
    "installationId",
    "agentId",
    "agentInstanceId",
    "signature",
}


def redact(value: Any) -> Any:
    if isinstance(value, dict):
        return {
            key: "[REDACTED]" if key in REDACT_KEYS else redact(item)
            for key, item in value.items()
        }
    if isinstance(value, list):
        return [redact(item) for item in value]
    return value


def reader(stream: Any, output: queue.Queue[str]) -> None:
    try:
        for line in iter(stream.readline, ""):
            output.put(line.rstrip("\r\n"))
    finally:
        stream.close()


def wait_for_id(output: queue.Queue[str], request_id: Any, timeout: float) -> dict[str, Any]:
    deadline = time.monotonic() + timeout
    observed: list[dict[str, Any]] = []
    while time.monotonic() < deadline:
        try:
            line = output.get(timeout=min(0.25, max(0.01, deadline - time.monotonic())))
        except queue.Empty:
            continue
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            continue
        observed.append(message)
        if message.get("id") == request_id:
            return {"response": message, "precedingMessages": observed[:-1]}
    raise TimeoutError(f"timed out waiting for response id {request_id!r}")


def wait_for_method(output: queue.Queue[str], method: str, timeout: float) -> dict[str, Any]:
    deadline = time.monotonic() + timeout
    observed: list[dict[str, Any]] = []
    while time.monotonic() < deadline:
        try:
            line = output.get(timeout=min(0.25, max(0.01, deadline - time.monotonic())))
        except queue.Empty:
            continue
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            continue
        observed.append(message)
        if message.get("method") == method:
            return {"terminal": message, "precedingMessages": observed[:-1]}
    raise TimeoutError(f"timed out waiting for method {method!r}")


def send(process: subprocess.Popen[str], message: dict[str, Any]) -> None:
    assert process.stdin is not None
    process.stdin.write(json.dumps(message, separators=(",", ":")) + "\n")
    process.stdin.flush()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("mode", choices=("grok-acp", "codex-app-server"))
    parser.add_argument("--binary", required=True)
    parser.add_argument("--home", required=True)
    parser.add_argument("--workspace", required=True)
    parser.add_argument("--timeout", type=float, default=20.0)
    parser.add_argument("--prompt")
    parser.add_argument("--auth-path")
    args = parser.parse_args()

    home = Path(args.home).resolve()
    workspace = Path(args.workspace).resolve()
    home.mkdir(parents=True, exist_ok=True)
    workspace.mkdir(parents=True, exist_ok=True)

    env = os.environ.copy()
    if args.mode == "grok-acp":
        env["GROK_HOME"] = str(home)
        if args.auth_path:
            env["GROK_AUTH_PATH"] = str(Path(args.auth_path).resolve())
        command = [args.binary, "agent", "stdio"]
    else:
        env["CODEX_HOME"] = str(home)
        command = [args.binary, "app-server", "--stdio"]

    process = subprocess.Popen(
        command,
        cwd=workspace,
        env=env,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        encoding="utf-8",
        errors="replace",
        bufsize=1,
    )
    assert process.stdout is not None
    assert process.stderr is not None
    stdout_queue: queue.Queue[str] = queue.Queue()
    stderr_queue: queue.Queue[str] = queue.Queue()
    threading.Thread(target=reader, args=(process.stdout, stdout_queue), daemon=True).start()
    threading.Thread(target=reader, args=(process.stderr, stderr_queue), daemon=True).start()

    result: dict[str, Any] = {
        "mode": args.mode,
        "binary": str(Path(args.binary).resolve()),
        "isolatedHome": str(home),
        "isolatedWorkspace": str(workspace),
        "modelPromptSubmitted": bool(args.prompt),
        "authenticationAttempted": bool(args.prompt and args.mode == "grok-acp"),
    }

    try:
        if args.mode == "grok-acp":
            send(
                process,
                {
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "initialize",
                    "params": {
                        "protocolVersion": 1,
                        "clientCapabilities": {
                            "fs": {"readTextFile": False, "writeTextFile": False},
                            "terminal": False,
                        },
                        "_meta": {
                            "startupHints": {
                                "nonInteractive": True,
                                "skipGitStatus": True,
                                "skipProjectLayout": True,
                            },
                            "clientType": "gogo-party-spike",
                            "clientVersion": "0.0.0",
                        },
                    },
                },
            )
            result["initialize"] = wait_for_id(stdout_queue, 1, args.timeout)
            if args.prompt:
                send(
                    process,
                    {
                        "jsonrpc": "2.0",
                        "id": 2,
                        "method": "authenticate",
                        "params": {"methodId": "grok.com", "_meta": {"headless": True}},
                    },
                )
                result["authenticate"] = wait_for_id(stdout_queue, 2, args.timeout)
                send(
                    process,
                    {
                        "jsonrpc": "2.0",
                        "id": 3,
                        "method": "session/new",
                        "params": {
                            "cwd": str(workspace),
                            "mcpServers": [],
                            "_meta": {"yoloMode": False},
                        },
                    },
                )
                result["sessionNew"] = wait_for_id(stdout_queue, 3, args.timeout)
                session_id = result["sessionNew"]["response"]["result"]["sessionId"]
                send(
                    process,
                    {
                        "jsonrpc": "2.0",
                        "id": 4,
                        "method": "session/prompt",
                        "params": {
                            "sessionId": session_id,
                            "prompt": [{"type": "text", "text": args.prompt}],
                        },
                    },
                )
                result["prompt"] = wait_for_id(stdout_queue, 4, args.timeout)
            else:
                send(
                    process,
                    {
                        "jsonrpc": "2.0",
                        "id": 2,
                        "method": "session/new",
                        "params": {"cwd": str(workspace), "mcpServers": []},
                    },
                )
                result["sessionNewWithoutAuth"] = wait_for_id(stdout_queue, 2, args.timeout)
        else:
            send(
                process,
                {
                    "id": 1,
                    "method": "initialize",
                    "params": {
                        "clientInfo": {
                            "name": "gogo_party_spike",
                            "title": "GOGO PARTY Adapter Spike",
                            "version": "0.0.0",
                        }
                    },
                },
            )
            result["initialize"] = wait_for_id(stdout_queue, 1, args.timeout)
            send(process, {"method": "initialized", "params": {}})
            send(
                process,
                {
                    "id": 2,
                    "method": "thread/start",
                    "params": {
                        "cwd": str(workspace),
                        "ephemeral": True,
                        "approvalPolicy": "never",
                        "sandbox": "read-only",
                    },
                },
            )
            result["threadStartEphemeral"] = wait_for_id(stdout_queue, 2, args.timeout)
            if args.prompt:
                thread_id = result["threadStartEphemeral"]["response"]["result"]["thread"]["id"]
                send(
                    process,
                    {
                        "id": 3,
                        "method": "turn/start",
                        "params": {
                            "threadId": thread_id,
                            "input": [{"type": "text", "text": args.prompt}],
                            "approvalPolicy": "never",
                            "sandboxPolicy": {"type": "readOnly"},
                        },
                    },
                )
                result["turnStart"] = wait_for_id(stdout_queue, 3, args.timeout)
                result["turnCompleted"] = wait_for_method(
                    stdout_queue, "turn/completed", args.timeout
                )
    except Exception as exc:
        result["probeError"] = f"{type(exc).__name__}: {exc}"
    finally:
        try:
            if process.stdin is not None:
                process.stdin.close()
        except OSError:
            pass
        try:
            process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            process.terminate()
            try:
                process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=3)

    stderr_lines: list[str] = []
    while not stderr_queue.empty():
        stderr_lines.append(stderr_queue.get_nowait())
    result["processExitCode"] = process.returncode
    result["stderrTail"] = stderr_lines[-40:]
    print(json.dumps(redact(result), ensure_ascii=False, indent=2))
    return 0 if "probeError" not in result else 1


if __name__ == "__main__":
    raise SystemExit(main())
