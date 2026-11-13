"""External programs and machine-local settings.

Settings are read from the JSON file named by ZONEKIT_SETTINGS, else from
local/zonekit/settings.json in the repository root (ignored by Git). See
docs/zonekit.md for the keys. No program or folder has a default path.
"""

import json
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(os.path.dirname(HERE)))
PY = sys.executable
CENSUS = os.path.join(REPO, "tools", "research", "census_wrb.py")
RESEARCH = os.path.join(REPO, "tools", "research")
_ENV = {
    "blender": "ZONEKIT_BLENDER",
    "xiv1tt": "ZONEKIT_XIV1TT",
    "phb_author": "ZONEKIT_PHB_AUTHOR",
    "zones": "ZONEKIT_ZONES",
}
_settings = None


def settings():
    global _settings
    if _settings is None:
        path = os.environ.get("ZONEKIT_SETTINGS") or os.path.join(
            REPO, "local", "zonekit", "settings.json"
        )
        _settings = {}
        if os.path.exists(path):
            with open(path, encoding="utf-8") as f:
                _settings = json.load(f)
    return _settings


def setting(key):
    """A program or folder path from the environment or the settings file; stops when unset."""
    value = os.environ.get(_ENV[key]) or settings().get(key)
    if not value:
        raise SystemExit(
            "zonekit: set '%s' in local/zonekit/settings.json or %s (docs/zonekit.md)"
            % (key, _ENV[key])
        )
    return value


def read_only():
    """Folders zonekit never writes under (client installs)."""
    return list(settings().get("read_only", []))


def blender_script(name):
    return os.path.join(HERE, "blender", name)


def run(cmd, ok=(0,)):
    """Run a program; stop with its output tail if it fails. Returns stdout."""
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode not in ok:
        raise SystemExit(f"FAILED {cmd}\n{r.stdout[-3000:]}\n{r.stderr[-3000:]}")
    return r.stdout
