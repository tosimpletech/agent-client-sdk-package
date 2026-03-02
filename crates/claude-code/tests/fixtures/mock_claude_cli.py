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
            "event": {
                "type": "content_block_delta",
                "delta": {"type": "text_delta", "text": "hi"},
            },
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


def parse_csv(value: str):
    if not value:
        return []
    return [item for item in value.split(",") if item]


def is_tool_allowed(tool_name: str, allowed_tools, disallowed_tools):
    if tool_name in disallowed_tools:
        return False
    if not allowed_tools:
        return False
    return tool_name in allowed_tools


def make_mcp_request(request_id: str, server_name: str, method: str, params: dict):
    return {
        "request_id": request_id,
        "request": {
            "subtype": "mcp_message",
            "server_name": server_name,
            "message": {
                "jsonrpc": "2.0",
                "id": request_id,
                "method": method,
                "params": params,
            },
        },
    }


def build_mcp_sequence(scenario: str, allowed_tools, disallowed_tools):
    sequence = []
    completion_text = "MCP response handled"

    if scenario == "list_only":
        sequence.append(make_mcp_request("mcp_req_1", "mock-sdk", "tools/list", {}))
        return sequence, completion_text

    if scenario == "tool_execution":
        server_name = "test"
        planned_calls = [
            ("mcp__test__echo", "echo", {"text": "hello from mock"}),
        ]
        completion_text = "SDK MCP tool_execution finished"
    elif scenario == "permission_enforcement":
        server_name = "test"
        planned_calls = [
            ("mcp__test__greet", "greet", {"name": "Alice"}),
            ("mcp__test__echo", "echo", {"text": "test"}),
        ]
        completion_text = "SDK MCP permission_enforcement finished"
    elif scenario == "multiple_tools":
        server_name = "multi"
        planned_calls = [
            ("mcp__multi__echo", "echo", {"text": "test"}),
            ("mcp__multi__greet", "greet", {"name": "Bob"}),
        ]
        completion_text = "SDK MCP multiple_tools finished"
    elif scenario == "without_permissions":
        server_name = "noperm"
        planned_calls = [
            ("mcp__noperm__echo", "echo", {"text": "blocked"}),
        ]
        completion_text = "SDK MCP without_permissions finished"
    else:
        return sequence, completion_text

    sequence.append(make_mcp_request("mcp_req_1", server_name, "tools/list", {}))
    request_index = 2
    for full_tool_name, short_name, args in planned_calls:
        if is_tool_allowed(full_tool_name, allowed_tools, disallowed_tools):
            sequence.append(
                make_mcp_request(
                    f"mcp_req_{request_index}",
                    server_name,
                    "tools/call",
                    {"name": short_name, "arguments": args},
                )
            )
            request_index += 1
    return sequence, completion_text


def main():
    argv = sys.argv[1:]
    if "-v" in argv or "--version" in argv:
        print(os.environ.get("MOCK_CLAUDE_VERSION", "2.1.0"))
        return 0

    flags, values = parse_flags(argv)
    include_partial = "include-partial-messages" in flags
    debug_to_stderr = "debug-to-stderr" in flags
    allowed_tools = parse_csv(values.get("allowedTools", ""))
    disallowed_tools = parse_csv(values.get("disallowedTools", ""))

    if debug_to_stderr:
        print("[DEBUG] mock cli boot", file=sys.stderr, flush=True)

    waiting_for_mcp_response = False
    current_mcp_request_id = None
    pending_mcp_sequence = []
    mcp_completion_text = "MCP response handled"

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
                    scenario = os.environ.get("MOCK_CLAUDE_MCP_SCENARIO", "list_only")
                    pending_mcp_sequence, mcp_completion_text = build_mcp_sequence(
                        scenario, allowed_tools, disallowed_tools
                    )
                    if pending_mcp_sequence:
                        waiting_for_mcp_response = True
                        next_req = pending_mcp_sequence.pop(0)
                        current_mcp_request_id = next_req["request_id"]
                        emit(
                            {
                                "type": "control_request",
                                "request_id": next_req["request_id"],
                                "request": next_req["request"],
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
            if req_id == current_mcp_request_id:
                if pending_mcp_sequence:
                    next_req = pending_mcp_sequence.pop(0)
                    current_mcp_request_id = next_req["request_id"]
                    emit(
                        {
                            "type": "control_request",
                            "request_id": next_req["request_id"],
                            "request": next_req["request"],
                        }
                    )
                else:
                    waiting_for_mcp_response = False
                    current_mcp_request_id = None
                    emit_assistant(mcp_completion_text)
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
