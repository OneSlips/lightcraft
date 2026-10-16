"""Export a trained checkpoint as LightCraft's AI Denoise model: `model.onnx` + `denoise-model.json`, then run
LightCraft's own installation self-test on them.

Offline training tooling; not part of LightCraft.

    python export.py --ckpt <run>/final.pt --out <folder> --rawdump <lc-denoise-rawdump> [--version 0.1.0]
"""

import argparse
import csv
import hashlib
import json
import os
import subprocess
import sys

import torch

import model as M

TILE, OVERLAP = 512, 64

NOTICE = (
    "LightCraft's own denoise model, trained by the LightCraft contributors. The weights are licensed MIT OR "
    "Apache-2.0, like LightCraft: you may use them for any purpose, including commercial work, and pass them on with "
    "this notice. Training pictures: {n} low-ISO raw samples from raw.pixls.us, each dedicated to the public domain "
    "(CC0) by its contributor; thank you."
)

PROVENANCE = (
    "Trained by LightCraft contributors with LightCraft's open training tools (tools/denoise-train) on {n} clean "
    "low-ISO raw photos from {cams} camera models published on raw.pixls.us under CC0, with synthetic sensor noise "
    "calibrated on real cameras (numbers only). No photos with other licences were used."
)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--ckpt", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--rawdump", required=True)
    ap.add_argument("--data", help="the data folder, for the credits in the manifest")
    ap.add_argument("--version", default="0.1.0")
    a = ap.parse_args()
    os.makedirs(a.out, exist_ok=True)
    s = torch.load(a.ckpt, map_location="cpu")
    net = M.BayerUNet(tuple(s.get("widths", (32, 64, 128, 256, 512))))
    net.load_state_dict(s["ema"])
    onnx_path = os.path.join(a.out, "model.onnx")
    M.export_onnx(net, onnx_path, TILE)
    data = open(onnx_path, "rb").read()

    n, cams = 0, 0
    if a.data and os.path.exists(os.path.join(a.data, "sources.csv")):
        rows = list(csv.DictReader(open(os.path.join(a.data, "sources.csv"), encoding="utf8")))
        assert all(r["licence"] == "CC0-1.0" for r in rows)
        n, cams = len(rows), len({(r["make"], r["model"]) for r in rows})

    manifest = {
        "id": "lightcraft-bayer",
        "name": "LightCraft Denoise (Bayer)",
        "version": a.version,
        "licence": {
            "name": "MIT OR Apache-2.0",
            "commercial": "yes",
            "url": "https://github.com/storytold/lightcraft/blob/main/LICENSE-MIT",
            "notice": NOTICE.format(n=n),
        },
        "source": "https://github.com/storytold/lightcraft/tree/main/tools/denoise-train",
        "sha256": hashlib.sha256(data).hexdigest(),
        "sizeBytes": len(data),
        "provenance": PROVENANCE.format(n=n, cams=cams),
        "domain": "bayerToRgb",
        "tile": TILE,
        "overlap": OVERLAP,
        "gain": {"kind": "none"},
    }
    man_path = os.path.join(a.out, "denoise-model.json")
    json.dump(manifest, open(man_path, "w", encoding="utf8"), indent=2)
    print(json.dumps({k: manifest[k] for k in ("id", "version", "sha256", "sizeBytes")}))
    p = subprocess.run([a.rawdump, "selftest", onnx_path, man_path], capture_output=True, text=True)
    print(p.stdout.strip(), p.stderr.strip())
    return p.returncode


if __name__ == "__main__":
    sys.exit(main())
