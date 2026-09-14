#!/usr/bin/env python3
"""Minimal `pi --mode rpc` stand-in for theseus-desktop tests."""

from __future__ import annotations

import json
import os
import sys
import time

SESSION_DIR = os.environ.get("THESEUS_FAKE_PI_DIR") or os.path.join(
    os.environ.get("TMPDIR") or os.environ.get("TEMP") or "/tmp",
    f"theseus-fake-pi-{os.getpid()}",
)
os.makedirs(SESSION_DIR, exist_ok=True)
SESSION_FILE = os.path.join(SESSION_DIR, "session.jsonl")

session_id = "sess_test"
session_name = ""
messages: list[dict] = []
streaming = False


def emit(obj: dict) -> None:
    sys.stdout.write(json.dumps(obj, separators=(",", ":")) + "\n")
    sys.stdout.flush()


def handle(cmd: dict) -> None:
    global session_id, session_name, messages, streaming
    typ = cmd.get("type")
    rid = cmd.get("id")
    if typ == "get_state":
        emit(
            {
                "type": "response",
                "id": rid,
                "command": "get_state",
                "success": True,
                "data": {
                    "model": {"id": "gpt-4o-mini", "name": "gpt-4o-mini"},
                    "isStreaming": streaming,
                    "sessionFile": SESSION_FILE,
                    "sessionId": session_id,
                    "sessionName": session_name or None,
                    "messageCount": len(messages),
                },
            }
        )
        return
    if typ == "new_session":
        session_id = "sess_new"
        session_name = ""
        messages = []
        emit(
            {
                "type": "response",
                "id": rid,
                "command": "new_session",
                "success": True,
                "data": {"cancelled": False},
            }
        )
        return
    if typ == "prompt":
        streaming = True
        msg = cmd.get("message") or ""
        messages.append({"role": "user", "content": msg, "timestamp": int(time.time() * 1000)})
        emit({"type": "response", "id": rid, "command": "prompt", "success": True})
        emit({"type": "agent_start"})
        emit({"type": "turn_start"})
        emit({"type": "message_start", "message": {"role": "user", "content": msg, "id": "m_user"}})
        emit({"type": "message_end", "message": {"role": "user", "content": msg, "id": "m_user"}})
        emit({"type": "message_start", "message": {"role": "assistant", "content": [], "id": "m_as"}})
        emit(
            {
                "type": "message_update",
                "assistantMessageEvent": {"type": "text_start", "contentIndex": 0},
            }
        )
        emit(
            {
                "type": "message_update",
                "assistantMessageEvent": {
                    "type": "text_delta",
                    "contentIndex": 0,
                    "delta": "pong",
                },
            }
        )
        emit(
            {
                "type": "message_update",
                "assistantMessageEvent": {
                    "type": "text_end",
                    "contentIndex": 0,
                    "content": "pong",
                },
            }
        )
        assistant = {
            "role": "assistant",
            "content": [{"type": "text", "text": "pong"}],
            "id": "m_as",
        }
        emit({"type": "message_end", "message": assistant})
        emit({"type": "turn_end", "message": assistant, "toolResults": []})
        streaming = False
        messages.append(assistant)
        emit({"type": "agent_settled"})
        return
    if typ == "abort":
        streaming = False
        emit({"type": "response", "id": rid, "command": "abort", "success": True})
        emit({"type": "agent_settled"})
        return
    if typ == "clear_queue":
        emit(
            {
                "type": "response",
                "id": rid,
                "command": "clear_queue",
                "success": True,
                "data": {"steering": [], "followUp": []},
            }
        )
        return
    if typ == "get_messages":
        emit(
            {
                "type": "response",
                "id": rid,
                "command": "get_messages",
                "success": True,
                "data": {"messages": messages},
            }
        )
        return
    if typ == "get_entries":
        emit(
            {
                "type": "response",
                "id": rid,
                "command": "get_entries",
                "success": True,
                "data": {"entries": [], "leafId": None},
            }
        )
        return
    if typ == "switch_session":
        emit(
            {
                "type": "response",
                "id": rid,
                "command": "switch_session",
                "success": True,
                "data": {"cancelled": False},
            }
        )
        return
    if typ == "set_session_name":
        session_name = cmd.get("name") or ""
        emit({"type": "response", "id": rid, "command": "set_session_name", "success": True})
        return
    emit(
        {
            "type": "response",
            "id": rid,
            "command": typ,
            "success": False,
            "error": f"unknown {typ}",
        }
    )


def main() -> None:
    if "--help" in sys.argv:
        sys.stdout.write("fake-pi --mode rpc\n")
        return
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            break
        if line.endswith(b"\n"):
            line = line[:-1]
        if line.endswith(b"\r"):
            line = line[:-1]
        if not line.strip():
            continue
        handle(json.loads(line.decode("utf-8")))


if __name__ == "__main__":
    main()
