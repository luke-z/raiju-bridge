#!/usr/bin/env python3
"""Translate GPUI's POSIX file paths before invoking FXC through Wine."""
import os
from pathlib import Path
import subprocess
import sys

root = Path(__file__).resolve().parent.parent
args = sys.argv[1:]
converted = []
for index, arg in enumerate(args):
    is_path = (index > 0 and args[index - 1] == "/Fh") or arg.endswith(".hlsl")
    if is_path:
        arg = subprocess.check_output(["winepath", "-w", str(Path(arg).resolve())], text=True).strip()
    converted.append(arg)
environment = dict(os.environ, WINEDLLOVERRIDES="d3dcompiler_47=n", WINEDEBUG="-all")
sys.exit(subprocess.call(["wine", str(root / ".build/fxc/fxc.exe"), *converted], env=environment))
