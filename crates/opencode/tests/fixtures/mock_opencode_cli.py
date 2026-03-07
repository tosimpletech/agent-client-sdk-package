#!/usr/bin/env python3
import json
import os
import signal
import sys
import time
from pathlib import Path


def parse_args(argv):
    flags = []
    values = {}
    for token in argv:
        if token.startswith("--") and "=" in token:
            key, value = token.split("=", 1)
            values[key] = value
            flags.append(key)
        elif token.startswith("--"):
            flags.append(token)
    return flags, values


def append_log(path: Path, payload: dict):
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a", encoding="utf-8") as f:
        f.write(json.dumps(payload, ensure_ascii=False) + "\n")


def main():
    args = sys.argv[1:]
    flags, values = parse_args(args)

    log_path = os.environ.get("OPENCODE_MOCK_LOG")
    if log_path:
        append_log(
            Path(log_path),
            {
                "args": args,
                "flags": flags,
                "values": values,
                "pid": os.getpid(),
                "env": {
                    "OPENCODE_CONFIG_CONTENT": os.environ.get("OPENCODE_CONFIG_CONTENT", ""),
                    "PATH": os.environ.get("PATH", ""),
                },
            },
        )

    if len(args) > 0 and args[0] == "serve":
        hostname = values.get("--hostname", "127.0.0.1")
        port = values.get("--port", "4096")
        no_listen = os.environ.get("OPENCODE_MOCK_NO_LISTEN") == "1"
        exit_log = os.environ.get("OPENCODE_MOCK_EXIT_LOG")
        if not no_listen:
            print(f"opencode server listening on http://{hostname}:{port}", flush=True)

        keep_running = True

        def handle_term(_signum, _frame):
            nonlocal keep_running
            keep_running = False

        signal.signal(signal.SIGTERM, handle_term)
        signal.signal(signal.SIGINT, handle_term)

        while keep_running:
            time.sleep(0.05)

        if exit_log:
            append_log(Path(exit_log), {"event": "serve-exit"})

        return

    # TUI/mock command mode
    time.sleep(1)


if __name__ == "__main__":
    main()
