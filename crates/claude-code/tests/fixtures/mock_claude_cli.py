#!/usr/bin/env python3
import json
import os
import sys


RESULT_MESSAGE = {
    "type": "result",
    "subtype": "success",
    "duration_ms": 10,
    "duration_api_ms": 5,
    "is_error": False,
    "num_turns": 1,
    "session_id": "mock-session",
    "total_cost_usd": 0.0,
}


def emit(obj):
    print(json.dumps(obj), flush=True)


def emit_assistant(text: str):
    emit(
        {
            "type": "assistant",
            "message": {
                "content": [{"type": "text", "text": text}],
                "model": "claude-sonnet-4-5",
            },
        }
    )


def emit_partial_events():
    emit(
        {
            "type": "stream_event",
            "uuid": "stream-1",
            "session_id": "mock-session",
            "event": {"type": "message_start"},
            "parent_tool_use_id": None,
        }
    )
    emit(
        {
            "type": "stream_event",
            "uuid": "stream-2",
            "session_id": "mock-session",
            "event": {"type": "content_block_delta", "delta": {"type": "text_delta", "text": "hi"}},
            "parent_tool_use_id": None,
        }
    )


def parse_flags(argv):
    flags = set()
    values = {}
    i = 0
    while i < len(argv):
        token = argv[i]
        if token.startswith("--"):
            key = token[2:]
            if i + 1 < len(argv) and not argv[i + 1].startswith("--"):
                values[key] = argv[i + 1]
                i += 2
                continue
            flags.add(key)
        i += 1
    return flags, values


def main():
    argv = sys.argv[1:]
    if "-v" in argv or "--version" in argv:
        print(os.environ.get("MOCK_CLAUDE_VERSION", "2.1.0"))
        return 0

    flags, _ = parse_flags(argv)
    include_partial = "include-partial-messages" in flags
    debug_to_stderr = "debug-to-stderr" in flags

    if debug_to_stderr:
        print("[DEBUG] mock cli boot", file=sys.stderr, flush=True)

    mcp_request_id = "mcp_req_1"
    waiting_for_mcp_response = False

    for raw in sys.stdin:
        line = raw.strip()
        if not line:
            continue

        try:
            msg = json.loads(line)
        except Exception:
            continue

        msg_type = msg.get("type")

        if msg_type == "control_request":
            req = msg.get("request", {})
            subtype = req.get("subtype")
            req_id = msg.get("request_id", "")

            if subtype == "initialize":
                emit(
                    {
                        "type": "control_response",
                        "response": {
                            "subtype": "success",
                            "request_id": req_id,
                            "response": {"ok": True},
                        },
                    }
                )
                if os.environ.get("MOCK_CLAUDE_TRIGGER_MCP") == "1":
                    waiting_for_mcp_response = True
                    emit(
                        {
                            "type": "control_request",
                            "request_id": mcp_request_id,
                            "request": {
                                "subtype": "mcp_message",
                                "server_name": "mock-sdk",
                                "message": {
                                    "jsonrpc": "2.0",
                                    "id": 42,
                                    "method": "tools/list",
                                    "params": {},
                                },
                            },
                        }
                    )
                continue

            if subtype in {
                "interrupt",
                "set_permission_mode",
                "set_model",
                "rewind_files",
                "mcp_status",
            }:
                emit(
                    {
                        "type": "control_response",
                        "response": {
                            "subtype": "success",
                            "request_id": req_id,
                            "response": {"ack": subtype},
                        },
                    }
                )
                continue

            emit(
                {
                    "type": "control_response",
                    "response": {
                        "subtype": "error",
                        "request_id": req_id,
                        "error": f"unsupported subtype: {subtype}",
                    },
                }
            )
            continue

        if msg_type == "control_response" and waiting_for_mcp_response:
            req_id = msg.get("response", {}).get("request_id")
            if req_id == mcp_request_id:
                waiting_for_mcp_response = False
                emit_assistant("MCP response handled")
                emit(RESULT_MESSAGE)
            continue

        if msg_type == "user":
            if waiting_for_mcp_response:
                # Keep the process alive until SDK sends MCP control_response.
                continue
            if debug_to_stderr:
                print("[DEBUG] got user message", file=sys.stderr, flush=True)
            if include_partial:
                emit_partial_events()
            emit_assistant("Mock answer")
            emit(RESULT_MESSAGE)
            continue

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
