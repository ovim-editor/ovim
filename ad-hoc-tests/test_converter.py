#!/usr/bin/env python3
import re
from pathlib import Path

def parse_snapshot(snapshot_path):
    try:
        with open(snapshot_path, 'r') as f:
            content = f.read()
    except FileNotFoundError:
        print(f"  Snapshot not found: {snapshot_path}")
        return None

    cursor_match = re.search(r'Cursor: (\d+):(\d+)', content)
    if not cursor_match:
        print(f"  No cursor info in: {snapshot_path}")
        return None

    cursor_line = int(cursor_match.group(1))
    cursor_col = int(cursor_match.group(2))

    buffer_match = re.search(r'Buffer:\n(.+?)(?:\n\n|\n---|\Z)', content, re.DOTALL)
    if not buffer_match:
        print(f"  No buffer in: {snapshot_path}")
        return None

    buffer_text = buffer_match.group(1).rstrip()
    buffer_lines = buffer_text.split('\n')

    processed_lines = []
    for line in buffer_lines:
        cleaned = re.sub(r'\[(.?)\]', r'\1', line)
        processed_lines.append(cleaned)

    buffer_content = '\n'.join(processed_lines) + '\n'
    return {'buffer': buffer_content, 'cursor_line': cursor_line, 'cursor_col': cursor_col}

# Test with one snapshot
snap = parse_snapshot(Path('/workspace/tests/snapshots/register_operations_test__yank_to_named_register.snap'))
if snap:
    print(f"Buffer: {repr(snap['buffer'])}")
    print(f"Cursor: {snap['cursor_line']}:{snap['cursor_col']}")
