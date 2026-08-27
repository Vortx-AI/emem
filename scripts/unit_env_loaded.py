#!/usr/bin/env python3
"""Every Environment= in the unit file must actually be loaded.

A restart re-runs the unit systemd holds in MEMORY. Edit the file, restart
without daemon-reload, and the service comes up healthy running the old
definition -- which is how four operator variables were added, deployed, and
served as absent while every check passed.

Also catches the second fault underneath that one: systemd splits an unquoted
Environment= value on whitespace, so `NAME=Vortx AI Private Limited` silently
becomes `NAME=Vortx`. The file and the loaded value differ, and only comparing
them shows it.
"""
import re
import subprocess
import sys
import pathlib

UNIT = pathlib.Path.home() / ".config/systemd/user/emem-server.service"


def declared() -> dict[str, str]:
    out = {}
    for line in UNIT.read_text().splitlines():
        line = line.strip()
        if not line.startswith("Environment="):
            continue
        body = line[len("Environment="):].strip()
        if body.startswith('"') and body.endswith('"'):
            body = body[1:-1]
        # strip a trailing inline comment only when the value is unquoted
        k, _, v = body.partition("=")
        out[k.strip()] = v.split("   #")[0].rstrip()
    return out


def loaded() -> dict[str, str]:
    raw = subprocess.run(
        ["systemctl", "--user", "show", "emem-server", "-p", "Environment", "--value"],
        capture_output=True, text=True, timeout=30).stdout.strip()
    # systemd prints assignments space-separated, quoting values that need it
    out = {}
    for m in re.finditer(r'"([^"]+)"|(\S+)', raw):
        item = m.group(1) or m.group(2)
        if "=" in item:
            k, _, v = item.partition("=")
            out[k] = v
    return out


def main() -> int:
    if not UNIT.exists():
        print(f"unit_env_loaded: {UNIT} does not exist; nothing to compare.")
        return 1
    d, l = declared(), loaded()
    print(f"  {len(d)} declared in the unit file, {len(l)} loaded by systemd")

    if not d:
        print("\nunit_env_loaded: VACUOUS -- parsed no Environment= line at all.")
        print("  This unit sets many. Zero means the parser broke, not the unit.")
        return 1

    bad = []
    specifiers = 0
    for k, v in d.items():
        if k not in l:
            bad.append(f"{k} is in the unit file and NOT loaded "
                       f"-- run: systemctl --user daemon-reload && systemctl --user restart emem-server")
        elif "%" in v:
            # systemd SPECIFIERS (%t, %h, %u, ...) are expanded at load time, so
            # the loaded value is SUPPOSED to differ from the file. The first
            # run of this gate reported EMEM_SIDECAR_SOCK as a fault because
            # %t had become /run/user/1000 -- correct behaviour, accused as a
            # defect. Presence is still checked; the value is not compared.
            specifiers += 1
        elif l[k] != v:
            bad.append(f"{k} differs: file has {v[:40]!r}, systemd loaded {l[k][:40]!r} "
                       f"(an unquoted value containing spaces splits)")
    if bad:
        print("\nunit_env_loaded: the running service is not the unit on disk.")
        for b in bad:
            print(f"  x {b}")
        return 1
    print(f"  Every declared variable is loaded with the value the file gives it "
          f"({specifiers} carried a systemd specifier, checked for presence only).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
