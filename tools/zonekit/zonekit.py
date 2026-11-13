#!/usr/bin/env python
"""Entry point: python -I zonekit.py <command> ...  (see README.md)."""

import os
import sys

sys.path.insert(
    0, os.path.dirname(os.path.abspath(__file__))
)  # -I does not add the script folder

from zonekit.cli import main

if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
