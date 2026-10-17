#!/usr/bin/env python3
"""Prepare fresh format-3 controls and append their measured SST sizes."""
import json

import pointindex_cost as cost

rows = json.loads((cost.OUT / "space.json").read_text())
assert not any(row["label"] == "off" for row in rows), "refusing to overwrite space controls"
cost.run.CPU = 8
cost.run.BINARY = cost.BIN / "after"
for layout in ["bulk", "normal"]:
    config = cost.run.config("kiban", 1000000, layout, 512)
    db = cost.DATA / "off" / ("kiban-" + layout)
    assert not db.exists(), "use fresh format-3 fixtures"
    config.update(db=str(db), point_index=False, action="prepare")
    cost.run.run(config, cost.OUT / "prepare", "off-kiban-" + layout)
    sizes = []
    for path in db.glob("*.sst"):
        with path.open("rb") as stream:
            stream.seek(-44, 2)
            footer = stream.read()
        assert int.from_bytes(footer[16:20], "little") == 3
        sizes.append(path.stat().st_size)
    assert sizes
    rows.append(dict(label="off", layout=layout, sst_bytes=sum(sizes),
                     point_bytes=0, files=len(sizes), bytes_per_key=0))
(cost.OUT / "space.json").write_text(json.dumps(rows, indent=2) + "\n")
