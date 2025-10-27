#!/usr/bin/env python3
"""
Parse LSP protocol output from rust-analyzer
Handles Content-Length headers and extracts JSON responses
"""

import sys
import json
import re

def parse_lsp_stream(content):
    """Parse LSP stream with Content-Length headers"""
    messages = []

    # Split by Content-Length headers
    parts = content.split('Content-Length:')

    for part in parts[1:]:  # Skip first empty part
        # Extract the length
        lines = part.split('\n', 1)
        if len(lines) < 2:
            continue

        try:
            length = int(lines[0].strip())
        except ValueError:
            continue

        # Find the JSON content (after the empty line)
        json_start = part.find('{')
        if json_start == -1:
            continue

        json_str = part[json_start:json_start + length]

        try:
            msg = json.loads(json_str)
            messages.append(msg)
        except json.JSONDecodeError as e:
            print(f"Warning: Failed to parse JSON: {e}", file=sys.stderr)
            print(f"Content: {json_str[:200]}...", file=sys.stderr)

    return messages

def main():
    if len(sys.argv) < 2:
        print("Usage: parse_lsp_output.py <log_file>")
        sys.exit(1)

    with open(sys.argv[1], 'r') as f:
        content = f.read()

    messages = parse_lsp_stream(content)

    print(f"=== Parsed {len(messages)} LSP messages ===\n")

    for i, msg in enumerate(messages, 1):
        print(f"Message {i}:")

        if 'method' in msg:
            # Request or notification
            print(f"  Type: {'Request' if 'id' in msg else 'Notification'}")
            print(f"  Method: {msg['method']}")
            if 'id' in msg:
                print(f"  ID: {msg['id']}")
        elif 'result' in msg or 'error' in msg:
            # Response
            print(f"  Type: Response")
            print(f"  ID: {msg.get('id', 'N/A')}")
            if 'result' in msg:
                result = msg['result']
                if result is None:
                    print(f"  Result: null")
                elif isinstance(result, dict):
                    if 'capabilities' in result:
                        print(f"  Result: Server capabilities (omitted for brevity)")
                    elif 'contents' in result:
                        # Hover response
                        print(f"  Result: Hover data")
                        print(f"    Contents: {json.dumps(result['contents'], indent=6)}")
                        if 'range' in result:
                            print(f"    Range: {result['range']}")
                    else:
                        print(f"  Result: {json.dumps(result, indent=4)}")
                else:
                    print(f"  Result: {result}")
            if 'error' in msg:
                print(f"  Error: {json.dumps(msg['error'], indent=4)}")

        print()

if __name__ == '__main__':
    main()
