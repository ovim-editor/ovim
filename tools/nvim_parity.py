#!/usr/bin/env python3
"""Run key sequences in nvim and in ovim and compare the results.

    tools/nvim_parity.py compare CASES   print every case where the two differ
    tools/nvim_parity.py rows CASES      print rows for ovim/tests/nvim_parity_test.rs

CASES is a text file with one case per line, `FIXTURE<TAB>keys`, where
FIXTURE names one of the texts below and keys use Vim notation (`<Esc>`,
`<C-v>`, `<CR>`). Lines starting with `#` are comments; `rows` copies them
into its output.

nvim runs as `nvim --clean --noplugin` and gets the keys without mappings, so
the bundled matchit plugin does not change what `%` does. It is told
`startofline expandtab shiftwidth=4`, which are ovim's defaults. ovim runs
headless with an empty home directory, so no user configuration applies.

Both must be built or installed first: `cargo build -p ovim`, and `nvim` on
PATH. Set OVIM to use another binary than target/debug/ovim.

Not everything a comparison reports is a bug:
- nvim leaves Insert mode when the keys end, ovim stays in it, so a case
  that ends in Insert mode differs in `mode`.
- In `ovim send`, `\\e`, `\\c` and `\\n` mean Escape, Ctrl-C and Enter. Write
  `\\\\c` for a literal backslash-c in a pattern.
- ovim puts the cursor back where it was before a change on undo; Vim puts
  it at the start of the change.
"""

import concurrent.futures
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OVIM = os.environ.get("OVIM", os.path.join(REPO, "target", "debug", "ovim"))

# Keep in step with the constants in ovim/tests/nvim_parity_test.rs.
FIXTURES = {
    "A": "one two three four five\nsecond (a, b) line\nthird\n",
    "B": 'fn main() {\n    let x = foo(bar, "baz qux");\n    if x { y(); }\n}\n\n'
    "last paragraph here.\nSecond sentence.  Third one? Yes.\n",
    "C": "alpha\n\n\nbeta gamma\n\tdelta  epsilon\nzeta-eta_theta 12 0x1f -7\n",
    "D": "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\n",
    "E": "foo.bar(baz[1], {k: v});\n<div class=\"x\">text</div>\n'single' `tick` \"dq\"\n",
}

NVIM_SETTINGS = "set startofline expandtab shiftwidth=4 tabstop=4 softtabstop=4"
NVIM_REPORT = (
    "local keys = vim.api.nvim_replace_termcodes(vim.env.KEYS, true, true, true) "
    'vim.api.nvim_feedkeys(keys, "ntx", false) '
    "local cursor = vim.api.nvim_win_get_cursor(0) "
    "io.stdout:write(vim.json.encode({"
    'cursor = {cursor[1] - 1, vim.fn.charcol(".") - 1}, mode = vim.fn.mode(), '
    "lines = vim.api.nvim_buf_get_lines(0, 0, -1, false)}))"
)
NVIM_MODES = {
    "n": "NORMAL",
    "i": "INSERT",
    "v": "VISUAL",
    "V": "VISUAL LINE",
    "\x16": "VISUAL BLOCK",
    "R": "REPLACE",
    "c": "COMMAND",
}


def run_nvim(text, keys, workdir):
    """(cursor, mode, text) after typing `keys` into `text` in nvim."""
    path = os.path.join(workdir, "nvim.txt")
    with open(path, "w") as file:
        file.write(text)
    result = subprocess.run(
        ["nvim", "--clean", "--noplugin", "--headless", "-n", "--cmd", NVIM_SETTINGS, path,
         "-c", "lua " + NVIM_REPORT, "-c", "qa!"],
        env=dict(os.environ, KEYS=keys),
        capture_output=True,
        text=True,
        timeout=30,
    )
    output = result.stdout if "{" in result.stdout else result.stderr
    report = json.loads(output[output.index("{"):])
    mode = NVIM_MODES.get(report["mode"], report["mode"])
    return tuple(report["cursor"]), mode, "\n".join(report["lines"]) + "\n"


def run_ovim(text, keys, workdir, session):
    """(cursor, mode, text) after typing `keys` into `text` in headless ovim."""
    path = os.path.join(workdir, "ovim.txt")
    with open(path, "w") as file:
        file.write(text)
    env = dict(os.environ, HOME=os.path.join(workdir, "home"))
    os.makedirs(env["HOME"], exist_ok=True)

    def ovim(*arguments):
        return subprocess.run([OVIM, *arguments, "-s", session], env=env,
                              capture_output=True, text=True, timeout=30)

    editor = subprocess.Popen([OVIM, path, "--headless", "--session", session], env=env,
                              stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        for _ in range(200):
            if ovim("session", "health").returncode == 0:
                break
            time.sleep(0.05)
        ovim("send", keys)
        snapshot = json.loads(ovim("snapshot", "--format", "json").stdout)
        buffer = ovim("buffer").stdout
        # `ovim buffer` prints the text and then a line break of its own.
        buffer = buffer[:-1] if buffer.endswith("\n") else buffer
        cursor = (snapshot["cursor"]["line"], snapshot["cursor"]["column"])
        return cursor, snapshot["mode"], buffer + ("" if buffer.endswith("\n") else "\n")
    finally:
        ovim("session", "kill")
        editor.kill()


def read_cases(path):
    """Yields ("comment", text) and ("case", (fixture, keys))."""
    with open(path) as file:
        for line in file:
            line = line.rstrip("\n")
            if line.startswith("#"):
                yield "comment", line[1:]
            elif line:
                fixture, keys = line.split("\t", 1)
                if fixture not in FIXTURES:
                    sys.exit(f"unknown fixture {fixture!r} in: {line}")
                yield "case", (fixture, keys)


def compare(path):
    cases = [case for kind, case in read_cases(path) if kind == "case"]

    def run(numbered):
        index, (fixture, keys) = numbered
        workdir = tempfile.mkdtemp(prefix="nvim-parity-")
        try:
            reference = run_nvim(FIXTURES[fixture], keys, workdir)
            actual = run_ovim(FIXTURES[fixture], keys, workdir, f"parity{os.getpid()}x{index}")
        finally:
            shutil.rmtree(workdir, ignore_errors=True)
        return fixture, keys, reference, actual

    different = 0
    with concurrent.futures.ThreadPoolExecutor(8) as pool:
        for fixture, keys, reference, actual in pool.map(run, enumerate(cases)):
            if reference == actual:
                continue
            different += 1
            print(f"--- [{fixture}] {keys}")
            if reference[:2] != actual[:2]:
                print(f"   nvim cursor={reference[0]} mode={reference[1]}"
                      f" | ovim cursor={actual[0]} mode={actual[1]}")
            if reference[2] != actual[2]:
                print(f"   nvim: {reference[2]!r}")
                print(f"   ovim: {actual[2]!r}")
    print(f"{different} of {len(cases)} differ")
    return 1 if different else 0


def rows(path):
    for kind, value in read_cases(path):
        if kind == "comment":
            print(f"        //{value}")
            continue
        fixture, keys = value
        workdir = tempfile.mkdtemp(prefix="nvim-parity-")
        try:
            cursor, _mode, text = run_nvim(FIXTURES[fixture], keys, workdir)
        finally:
            shutil.rmtree(workdir, ignore_errors=True)
        literal = lambda value: json.dumps(value, ensure_ascii=False)
        print(f"        ({fixture}, {literal(keys)}, {literal(text)}, ({cursor[0]}, {cursor[1]})),")
    return 0


if __name__ == "__main__":
    if len(sys.argv) != 3 or sys.argv[1] not in ("compare", "rows"):
        sys.exit(__doc__)
    sys.exit({"compare": compare, "rows": rows}[sys.argv[1]](sys.argv[2]))
