#!/usr/bin/env python3
"""
Parse insta snapshot files and extract buffer content and cursor position.
"""
import re
import sys
from pathlib import Path

def parse_snapshot(snapshot_path):
    """Parse a snapshot file and return (buffer_content, cursor_line, cursor_col, mode)"""
    with open(snapshot_path, 'r') as f:
        content = f.read()

    # Extract cursor line from "Cursor: LINE:COL"
    cursor_match = re.search(r'Cursor: (\d+):(\d+)', content)
    if not cursor_match:
        return None

    cursor_line = int(cursor_match.group(1))
    cursor_col = int(cursor_match.group(2))

    # Extract mode
    mode_match = re.search(r'Mode: (\w+)', content)
    mode = mode_match.group(1) if mode_match else "Normal"

    # Extract buffer section
    buffer_match = re.search(r'Buffer:\n(.+?)(?:\n\n|\Z)', content, re.DOTALL)
    if not buffer_match:
        return None

    buffer_lines = buffer_match.group(1).strip().split('\n')

    # Process each line to remove cursor markers [c]
    processed_lines = []
    for line in buffer_lines:
        # Remove cursor markers like [c] or [ ]
        cleaned = re.sub(r'\[(.?)\]', r'\1', line)
        processed_lines.append(cleaned)

    # Join with newlines and add final newline
    buffer_content = '\n'.join(processed_lines) + '\n'

    return {
        'buffer': buffer_content,
        'cursor_line': cursor_line,
        'cursor_col': cursor_col,
        'mode': mode
    }

def main():
    if len(sys.argv) < 2:
        print("Usage: parse_snapshots.py <snapshot_file>")
        sys.exit(1)

    snapshot_path = sys.argv[1]
    result = parse_snapshot(snapshot_path)

    if result:
        print(f"Buffer: {repr(result['buffer'])}")
        print(f"Cursor: {result['cursor_line']}:{result['cursor_col']}")
        print(f"Mode: {result['mode']}")
    else:
        print("Failed to parse snapshot")
        sys.exit(1)

if __name__ == '__main__':
    main()
