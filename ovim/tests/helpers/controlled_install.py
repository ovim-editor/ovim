"""An installer that records entry and waits until explicitly released."""

import os
import pathlib
import sys
import time

root = pathlib.Path(sys.argv[1])
name = sys.argv[2]
(root / f"{name}.pid").write_text(str(os.getpid()))
while root.exists() and not (root / f"{name}.release").exists():
    time.sleep(0.01)
