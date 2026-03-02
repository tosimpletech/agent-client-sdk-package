#!/usr/bin/env python3
import json
import os
import sys
from pathlib import Path


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


def emit_system_init(extra_data=None):
    payload = {
        "type": "system",
        "subtype": "init",
        "session_id": "mock-session",
    }
    if isinstance(extra_data, dict):
        payload.update(extra_data)
    emit(payload)


def emit_assistant(text: str):
    emit_assistant_blocks([{"type": "text", "text": text}])


def emit_assistant_blocks(content):
    emit(
        {
            "type": "assistant",
            "message": {
                "content": content,
                "model": "claude-sonnet-4-5",
            },
        }
    )


def emit_result(structured_output=None):
    result = dict(RESULT_MESSAGE)
    if structured_output is not None:
        result["structured_output"] = structured_output
    emit(result)


def emit_stream_event(index: int, event: dict):
    emit(
        {
            "type": "stream_event",
            "uuid": f"stream-{index}",
            "session_id": "mock-session",
            "event": event,
            "parent_tool_use_id": None,
        }
    )


def emit_partial_events():
    events = [
        {
            "type": "message_start",
            "message": {
                "id": "msg_1",
                "type": "message",
                "role": "assistant",
                "content": [],
                "model": "claude-sonnet-4-5",
            },
        },
        {
            "type": "content_block_start",
            "index": 0,
            "content_block": {"type": "thinking", "thinking": "", "signature": "sig-stream"},
        },
        {
            "type": "content_block_delta",
            "index": 0,
            "delta": {"type": "thinking_delta", "thinking": "2 + 2 means adding two and two. "},
        },
        {
            "type": "content_block_delta",
            "index": 0,
            "delta": {"type": "thinking_delta", "thinking": "That sum is 4 based on basic arithmetic."},
        },
        {"type": "content_block_stop", "index": 0},
        {
            "type": "content_block_start",
            "index": 1,
            "content_block": {"type": "text", "text": ""},
        },
        {
            "type": "content_block_delta",
            "index": 1,
            "delta": {"type": "text_delta", "text": "The answer is 4."},
        },
        {"type": "content_block_stop", "index": 1},
        {"type": "message_stop"},
    ]

    for i, event in enumerate(events, start=1):
        emit_stream_event(i, event)


def emit_tool_use_sequence():
    emit(
        {
            "type": "assistant",
            "message": {
                "content": [
                    {
                        "type": "tool_use",
                        "id": "toolu_mock_1",
                        "name": "Glob",
                        "input": {"pattern": "*"},
                    }
                ],
                "model": "claude-sonnet-4-5",
            },
        }
    )
    emit(
        {
            "type": "user",
            "message": {
                "content": [
                    {
                        "type": "tool_result",
                        "tool_use_id": "toolu_mock_1",
                        "content": {"matches": ["README.md", "src/lib.rs"]},
                    }
                ]
            },
        }
    )
    emit_assistant("Tool analysis complete")


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


def parse_setting_sources(raw_value):
    if not raw_value:
        return set()
    return {source.strip() for source in raw_value.split(",") if source.strip()}


def parse_json_schema(values):
    raw_schema = values.get("json-schema")
    if not raw_schema:
        return None

    try:
        parsed = json.loads(raw_schema)
    except Exception:
        return None

    return parsed if isinstance(parsed, dict) else None


def read_json_file(path: Path):
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except Exception:
        return None


def parse_frontmatter_name(path: Path):
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except Exception:
        return None

    if not lines or lines[0].strip() != "---":
        return None

    for line in lines[1:]:
        stripped = line.strip()
        if stripped == "---":
            break
        if not stripped.startswith("name:"):
            continue

        name = stripped.split(":", 1)[1].strip()
        if len(name) >= 2 and name[0] == name[-1] and name[0] in {'"', "'"}:
            name = name[1:-1]
        return name or None

    return None


def load_filesystem_state(cwd: Path, setting_sources):
    claude_dir = cwd / ".claude"
    agents = []
    slash_commands = []
    output_style = "default"

    if "project" in setting_sources:
        agents_dir = claude_dir / "agents"
        if agents_dir.exists():
            for agent_file in sorted(agents_dir.glob("*.md")):
                agent_name = parse_frontmatter_name(agent_file) or agent_file.stem
                agents.append(agent_name)

        commands_dir = claude_dir / "commands"
        if commands_dir.exists():
            for command_file in sorted(commands_dir.glob("*.md")):
                slash_commands.append(command_file.stem)

    if "local" in setting_sources:
        local_settings_path = claude_dir / "settings.local.json"
        local_settings = read_json_file(local_settings_path)
        if isinstance(local_settings, dict):
            candidate = local_settings.get("outputStyle")
            if isinstance(candidate, str) and candidate:
                output_style = candidate

    return {
        "agents": agents,
        "slash_commands": slash_commands,
        "output_style": output_style,
    }


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


def build_structured_output(schema):
    if not isinstance(schema, dict):
        return None

    properties = schema.get("properties")
    if not isinstance(properties, dict):
        return None

    if "analysis" in properties and "words" in properties:
        return {
            "analysis": {"word_count": 2, "character_count": 11},
            "words": ["Hello", "world"],
        }

    if "test_framework" in properties:
        framework = "pytest"
        enum_values = None
        framework_prop = properties.get("test_framework")
        if isinstance(framework_prop, dict):
            enum_values = framework_prop.get("enum")
        if isinstance(enum_values, list) and enum_values:
            if "pytest" in enum_values:
                framework = "pytest"
            elif "unknown" in enum_values:
                framework = "unknown"
            elif isinstance(enum_values[0], str):
                framework = enum_values[0]

        return {
            "has_tests": True,
            "test_framework": framework,
            "test_count": 4,
        }

    if "has_readme" in properties:
        return {"file_count": 3, "has_readme": True}

    if "has_tests" in properties:
        payload = {"file_count": 7, "has_tests": True}
        if "test_file_count" in properties:
            payload["test_file_count"] = 2
        return payload

    return {"ok": True}


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
    setting_sources = parse_setting_sources(values.get("setting-sources", ""))
    json_schema = parse_json_schema(values)
    cwd = Path(os.getcwd())

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
                initialize_agents = req.get("agents")
                initialize_agent_names = []
                initialize_agent_bytes = 0
                if isinstance(initialize_agents, dict):
                    initialize_agent_names = sorted(
                        name for name in initialize_agents.keys() if isinstance(name, str)
                    )
                if initialize_agents is not None:
                    try:
                        initialize_agent_bytes = len(json.dumps(initialize_agents))
                    except Exception:
                        initialize_agent_bytes = 0

                fs_state = load_filesystem_state(cwd, setting_sources)
                all_agents = sorted(set(initialize_agent_names + fs_state["agents"]))

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
                emit_system_init(
                    {
                        "agents": all_agents,
                        "slash_commands": fs_state["slash_commands"],
                        "output_style": fs_state["output_style"],
                        "initialize_has_agents": bool(initialize_agent_names),
                        "initialize_agent_bytes": initialize_agent_bytes,
                        "argv_has_agents_flag": "--agents" in argv,
                        "setting_sources": sorted(setting_sources),
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
                    emit_result()
            continue

        if msg_type == "user":
            if waiting_for_mcp_response:
                # Keep the process alive until SDK sends MCP control_response.
                continue
            if debug_to_stderr:
                print("[DEBUG] got user message", file=sys.stderr, flush=True)

            emit_system_init()

            if include_partial:
                emit_partial_events()
                emit_assistant_blocks(
                    [
                        {
                            "type": "thinking",
                            "thinking": "2 + 2 equals 4 after straightforward addition.",
                            "signature": "sig-final",
                        },
                        {"type": "text", "text": "The answer is 4."},
                    ]
                )
                emit_result()
                continue

            structured_output = build_structured_output(json_schema)
            if (
                structured_output is not None
                and os.environ.get("MOCK_CLAUDE_STRUCTURED_WITH_TOOLS") == "1"
            ):
                emit_tool_use_sequence()
                emit_result(structured_output)
                continue

            if structured_output is not None:
                emit_assistant("Mock structured answer")
                emit_result(structured_output)
                continue

            emit_assistant("Mock answer")
            emit_result()
            continue

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
