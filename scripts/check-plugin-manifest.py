import pathlib
import tomllib

m = tomllib.loads(pathlib.Path("herdr-plugin.toml").read_text())
for key in ("id", "name", "version", "min_herdr_version"):
    assert m.get(key), f"herdr-plugin.toml is missing {key}"
ids = [a["id"] for a in m.get("actions", [])]
assert len(ids) == len(set(ids)), f"duplicate action ids: {ids}"
for entry in m.get("build", []) + m.get("startup", []) + m.get("actions", []) + m.get("panes", []):
    cmd = entry["command"]
    assert isinstance(cmd, list) and cmd, "commands must be argv arrays"
    if cmd[0] in ("sh", "bash"):
        script = pathlib.Path(cmd[1])
        assert script.is_file(), f"manifest points at a missing script: {script}"
assert set(m["platforms"]) == {"linux", "macos"}, "install.sh supports linux and macos only"
print(f"ok: {m['id']} {m['version']}, floor {m['min_herdr_version']}, {len(ids)} actions")
