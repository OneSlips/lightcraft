#!/usr/bin/env python3
"""Does the photo on screen stay as quick while Build Previews runs? (design of 2026-10-31, §8)

usage: tools/drag-during-build.py PORT [--trials 3] [--seconds 3] [--size full|standard] [--label NAME]

Drives a running LightCraft over its control channel (JSON lines). Start it with its own library
and settings first, e.g.
  LIGHTCRAFT_NO_PREFS=1 lightcraft --control 18600 --memory
It opens the active photo in the loupe and drags Exposure (one undo step per drag, checked against
the starting value) while the machine is idle, then again while a forced Build Previews of every
photo in view runs (cancelled at the end), and prints both:

- `p95LatencyMs`: 95th percentile of the drafts' request-to-screen time (`perf.loupeRenders`,
  measured by the app on the frames it draws);
- `maxUpdateMs`: the slowest UI frame seen during the drags (`perf.updateMs` per poll; the app's
  own `perf.maxUpdateMs`, a maximum since it started, is shown as `appMaxUpdateMs`);
- `renders`: the app's renders by who they were for (`library.previewProgress` → `renders`): the
  build's are background work, which goes to the second graphics card first; the loupe's never;
- `gpu`: `app.gpu` (each card and what it is doing, when the app reports it).

Pass: the loaded numbers are within 10 % of the idle ones (`--slack-ms` absolute slack for the
noise of very small numbers, default 2 ms) and the build rendered photos meanwhile. Exit status 1
otherwise (or when an undo does not restore Exposure). Writes LABEL.json with everything.
"""
import json, socket, statistics, sys, time

PORT = int(sys.argv[1]) if len(sys.argv) > 1 and not sys.argv[1].startswith("--") else 18600
args = sys.argv[1:]


def opt(name, default):
    return type(default)(args[args.index(name) + 1]) if name in args else default


TRIALS = opt("--trials", 3)
SECONDS = opt("--seconds", 3.0)
SIZE = opt("--size", "full")
LABEL = opt("--label", "drag-during-build")
SLACK_MS = opt("--slack-ms", 2.0)


class Conn:
    def __init__(self):
        self.s = socket.create_connection(("127.0.0.1", PORT), timeout=120)
        self.s.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        self.f = self.s.makefile("r", encoding="utf-8")
        self.n = 0

    def call(self, method, params=None):
        self.n += 1
        msg = {"id": self.n, "method": method}
        if params is not None:
            msg["params"] = params
        self.s.sendall((json.dumps(msg) + "\n").encode())
        while True:
            line = self.f.readline()
            if not line:
                raise RuntimeError("connection closed")
            r = json.loads(line)
            if r.get("id") == self.n:
                if not r.get("ok", False):
                    raise RuntimeError(f"{method}: {r.get('error')}")
                return r.get("result")

    def ex(self, command, params=None):
        return self.call("engine.execute", {"command": command, "params": params or {}})


def exposure(c):
    return c.ex("photo.inspect")["develop"]["light"]["exposure"]


def p95(xs):
    xs = sorted(xs)
    if not xs:
        return None
    return xs[min(len(xs) - 1, int(round(0.95 * (len(xs) - 1))))]


def drag(c):
    """One Exposure drag of SECONDS, undone. Returns its draft latencies and slowest frames."""
    start = exposure(c)
    t_now = c.call("ui.inspect")["perf"].get("nowMs") or 0
    c.ex("develop.beginInteraction", {"label": "drag-during-build"})
    t0 = time.perf_counter()
    k, frames, max_update = 0, [], 0.0
    while time.perf_counter() - t0 < SECONDS:
        k += 1
        c.ex("develop.set", {"values": {"light.exposure": round(start + 0.02 * k, 3)}})
        perf = c.call("ui.inspect")["perf"]
        frames.append(perf.get("updateMs") or 0)
        max_update = max(max_update, perf.get("maxUpdateMs") or 0)
        time.sleep(0.033)
    c.ex("develop.endInteraction")
    time.sleep(1.0)
    renders = [r for r in (c.call("ui.inspect")["perf"].get("loupeRenders") or []) if r.get("atMs", 0) > t_now]
    c.ex("edit.undo")
    time.sleep(0.3)
    back = exposure(c)
    if abs(back - start) > 1e-6:
        raise RuntimeError(f"undo did not restore Exposure ({start} → {back})")
    drafts = [r["latencyMs"] for r in renders if r.get("draft")]
    return {"steps": k, "draftLatencies": drafts, "allLatencies": [r["latencyMs"] for r in renders], "frameMsMax": max(frames or [0]), "maxUpdateMs": max_update}


def phase(c, name):
    runs = [drag(c) for _ in range(TRIALS)]
    lat = [x for r in runs for x in (r["draftLatencies"] or r["allLatencies"])]
    out = {
        "phase": name,
        "drafts": len(lat),
        "p95LatencyMs": p95(lat),
        "medianLatencyMs": statistics.median(lat) if lat else None,
        "maxUpdateMs": max(r["frameMsMax"] for r in runs),
        "appMaxUpdateMs": max(r["maxUpdateMs"] for r in runs),
        "runs": runs,
    }
    print(json.dumps({k: v for k, v in out.items() if k != "runs"}))
    return out


def gpu(c):
    try:
        return c.ex("app.gpu")
    except RuntimeError as e:
        return {"error": str(e)}


def within(loaded, idle):
    if loaded is None or idle is None:
        return False
    return loaded <= idle * 1.10 + SLACK_MS


def main():
    c = Conn()
    if "--helper" in args:  # (the second graphics card for this run: auto, off, same, a card's name)
        c.ex("app.gpu", {"helper": opt("--helper", "auto"), "wait": True})
    # (the photo dragged is the one the loupe shows: the active photo, selected alone)
    active = c.ex("library.state")["selection"]["active"]
    c.ex("library.select", {"ids": [active]})
    c.ex("view.detail")
    t0 = time.perf_counter()
    while (c.ex("library.previewProgress") or {}).get("running") and time.perf_counter() - t0 < 60:
        time.sleep(0.5)  # (an earlier build still finishing)
    time.sleep(2.0)
    idle = phase(c, "idle")
    before = c.ex("library.previewProgress")
    r = c.ex("library.buildPreviews", {"size": SIZE, "force": True})
    total = r.get("total", 0)
    t0 = time.perf_counter()
    while (c.ex("library.previewProgress") or {}).get("done", 0) < 1 and time.perf_counter() - t0 < 30:
        time.sleep(0.1)
    gpu_during = gpu(c)
    loaded = phase(c, "duringBuild")
    progress = c.ex("library.previewProgress") or {}
    gpu_after = gpu(c)
    c.ex("library.cancelPreviews")
    rendered = (progress.get("renders") or {}).get("background", 0) - ((before or {}).get("renders") or {}).get("background", 0)
    ok_latency = within(loaded["p95LatencyMs"], idle["p95LatencyMs"])
    ok_frames = within(loaded["maxUpdateMs"], idle["maxUpdateMs"])
    result = {
        "label": LABEL,
        "pass": bool(ok_latency and ok_frames and rendered > 0),
        "p95LatencyMs": {"idle": idle["p95LatencyMs"], "duringBuild": loaded["p95LatencyMs"], "ok": ok_latency},
        "maxUpdateMs": {"idle": idle["maxUpdateMs"], "duringBuild": loaded["maxUpdateMs"], "ok": ok_frames},
        "build": {"photos": total, "doneDuringDrags": progress.get("done"), "backgroundRenders": rendered, "renders": progress.get("renders")},
        "gpuDuringBuild": gpu_during,
        "gpuAfter": gpu_after,
    }
    print(json.dumps({k: v for k, v in result.items() if not k.startswith("gpu")}, indent=1))
    with open(f"{LABEL}.json", "w", encoding="utf-8") as f:
        json.dump({**result, "idle": idle, "duringBuild": loaded}, f, indent=1)
    return 0 if result["pass"] else 1


if __name__ == "__main__":
    sys.exit(main())
