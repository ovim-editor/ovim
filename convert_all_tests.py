#!/usr/bin/env python3
import re
from pathlib import Path

def parse_snapshot(snapshot_path):
    """Parse snapshot and extract buffer/cursor info"""
    try:
        with open(snapshot_path, 'r') as f:
            content = f.read()
    except FileNotFoundError:
        return None

    cursor_match = re.search(r'Cursor: (\d+):(\d+)', content)
    if not cursor_match:
        return None

    cursor_line = int(cursor_match.group(1))
    cursor_col = int(cursor_match.group(2))

    buffer_match = re.search(r'Buffer:\n(.+?)(?:\n\n|\n---|\Z)', content, re.DOTALL)
    if not buffer_match:
        return None

    buffer_text = buffer_match.group(1).rstrip()
    buffer_lines = buffer_text.split('\n')

    processed_lines = []
    for line in buffer_lines:
        cleaned = re.sub(r'\[(.?)\]', r'\1', line)
        processed_lines.append(cleaned)

    buffer_content = '\n'.join(processed_lines) + '\n'
    return {'buffer': buffer_content, 'cursor_line': cursor_line, 'cursor_col': cursor_col}

def convert_file(input_path):
    """Convert test file"""
    with open(input_path, 'r') as f:
        lines = f.readlines()

    output_lines = []
    i = 0
    test_name_pattern = re.compile(r'^fn\s+(\w+)\(')
    snapshot_dir = Path('/workspace/tests/snapshots')

    test_file_base = input_path.stem

    while i < len(lines):
        line = lines[i]

        # Skip the use insta line
        if 'use insta::assert_snapshot' in line:
            i += 1
            continue

        # Check if this line has assert_snapshot
        if 'assert_snapshot!(test.snapshot_state());' in line:
            # Find the current test function name by looking backwards
            test_name = None
            for j in range(i - 1, max(0, i - 50), -1):
                match = test_name_pattern.search(lines[j])
                if match:
                    test_name = match.group(1)
                    break

            if test_name:
                snapshot_name = f"{test_file_base}__{test_name}.snap"
                snapshot_path = snapshot_dir / snapshot_name
                snap_data = parse_snapshot(snapshot_path)

                if snap_data:
                    # Get the indentation from the current line
                    indent = line[:len(line) - len(line.lstrip())]

                    # Escape the buffer content for Rust string literal
                    buffer_escaped = snap_data['buffer'].replace('\\', '\\\\').replace('"', '\\"').replace('\n', '\\n')

                    # Replace with new assertions
                    output_lines.append(f'{indent}assert_eq!(test.buffer_content(), "{buffer_escaped}");\n')
                    output_lines.append(f'{indent}test.assert_cursor({snap_data["cursor_line"]}, {snap_data["cursor_col"]});\n')
                    i += 1
                    continue

        output_lines.append(line)
        i += 1

    return ''.join(output_lines)

# Process files
for test_file in ['register_operations_test.rs', 'mark_test.rs']:
    input_path = Path(f'/workspace/tests/{test_file}')
    print(f"Converting {test_file}...")

    converted = convert_file(input_path)

    # Count conversions
    original_count = open(input_path).read().count('assert_snapshot!')
    new_count = converted.count('assert_snapshot!')
    converted_count = original_count - new_count

    print(f"  Converted {converted_count} of {original_count} tests")

    # Write output
    with open(input_path, 'w') as f:
        f.write(converted)

print("\nDone!")
