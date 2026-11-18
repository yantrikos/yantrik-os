#!/usr/bin/env python3
"""Time the reader started per look against the reader kept running, on drawn screens.

usage: reader-bench.py <yantrik-ocr> <models dir> <screens dir> [<old yantrik-ocr>]
Prints one JSON report. Reads only the images in <screens dir>.
"""
import difflib
import json
import os
import subprocess
import sys
import time

BIN, MODELS, SCREENS = sys.argv[1:4]
OLD = sys.argv[4] if len(sys.argv) > 4 else None
ORDER = ["a", "b", "c", "d", "e", "a"]
WHAT = {
    0: "first look (nothing remembered)",
    1: "one new line, everything scrolled up",
    2: "a command typed at the prompt",
    3: "nothing changed",
    4: "a different screen altogether",
    5: "back to the first screen",
}


def image(name):
    return os.path.join(SCREENS, name + ".png")


def truth(name):
    with open(os.path.join(SCREENS, name + ".txt")) as text:
        return [line.rstrip("\n") for line in text]


def squash(text):
    return "".join(text.split())


def accuracy(lines, name):
    """How much of the screen's own text was read: character similarity, and whole lines right."""
    want = truth(name)
    got = [squash(line["text"]) for line in lines]
    exact = sum(1 for line in want if squash(line) in set(got))
    # Each line of the screen against the line read that is most like it.
    alike = [max((difflib.SequenceMatcher(None, squash(line), g, autojunk=False).ratio() for g in got), default=0.0) for line in want]
    return {"lines_wanted": len(want), "lines_read": len(got), "lines_exact": exact, "characters": round(sum(alike) / max(len(alike), 1), 4)}


def once(binary, name):
    started = time.perf_counter()
    out = subprocess.run([binary, "--models", MODELS, "--max-lines", "300", image(name)], capture_output=True, text=True, timeout=180)
    wall = time.perf_counter() - started
    if out.returncode != 0:
        raise RuntimeError(out.stderr.strip())
    return json.loads(out.stdout), wall


def cpu_seconds(pid):
    with open(f"/proc/{pid}/stat") as stat:
        fields = stat.read().rsplit(")", 1)[1].split()
    return (int(fields[11]) + int(fields[12])) / os.sysconf("SC_CLK_TCK")


def rss_mb(pid):
    with open(f"/proc/{pid}/status") as status:
        for line in status:
            if line.startswith("VmRSS:"):
                return int(line.split()[1]) // 1024
    return None


def main():
    report = {"cpu": open("/proc/cpuinfo").read().split("model name")[1].split(":")[1].split("\n")[0].strip()}

    # 1. As it is today: a new process for every look, which reads everything.
    fresh = {}
    per_look = []
    for step, name in enumerate(ORDER):
        answer, wall = once(BIN, name)
        fresh[name] = answer
        per_look.append({"screen": name, "what": WHAT[step], "wall_s": round(wall, 2), "cost": answer.get("cost"), "accuracy": accuracy(answer["lines"], name)})
    report["started_per_look"] = per_look
    # A screen that is not a terminal: a light form with type of several sizes, and buttons.
    answer, wall = once(BIN, "f")
    fresh["f"] = answer
    report["form"] = {"wall_s": round(wall, 2), "accuracy": accuracy(answer["lines"], "f"), "read": [line["text"] for line in answer["lines"]]}
    if OLD:
        report["installed_reader"] = {}
        for name in ("a", "e", "f"):
            answer, wall = once(OLD, name)
            report["installed_reader"][name] = {"wall_s": round(wall, 2), "accuracy": accuracy(answer["lines"], name)}
        report["installed_reader"]["f"]["read"] = [line["text"] for line in answer["lines"]]

    # 2. Kept running: the models loaded once, and what each line said remembered.
    started = time.perf_counter()
    reader = subprocess.Popen([BIN, "--models", MODELS, "--idle-secs", "60", "--serve"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)

    def ask(request):
        begun = time.perf_counter()
        reader.stdin.write(json.dumps(request) + "\n")
        reader.stdin.flush()
        return json.loads(reader.stdout.readline()), time.perf_counter() - begun

    kept = []
    for step, name in enumerate(ORDER):
        cpu_before = cpu_seconds(reader.pid)
        answer, wall = ask({"image": image(name), "space": "desktop", "max_lines": 300})
        if "error" in answer:
            raise RuntimeError(answer["error"])
        kept.append({
            "screen": name,
            "what": WHAT[step],
            "wall_s": round(wall, 3),
            "cpu_s": round(cpu_seconds(reader.pid) - cpu_before, 2),
            "cost": answer["cost"],
            "accuracy": accuracy(answer["lines"], name),
            # Memory may make a reading faster. It must not make it say anything else.
            "same_as_a_fresh_reading": answer["lines"] == fresh[name]["lines"],
        })
    report["kept_running"] = kept
    report["kept_running_first_answer_after_start_s"] = round(kept[0]["wall_s"] + 0, 3)
    report["reader_rss_mb"] = rss_mb(reader.pid)

    # 3. Another display has a memory of its own: the same screen there is read afresh.
    answer, wall = ask({"image": image("a"), "space": "mind-view", "max_lines": 300})
    report["same_screen_on_another_display"] = {"wall_s": round(wall, 3), "cost": answer["cost"]}

    # 4. Forgetting: the next look reads everything again.
    forgot, _ = ask({"forget": True})
    answer, wall = ask({"image": image("a"), "space": "desktop", "max_lines": 300})
    report["after_forget"] = {"forgot": forgot, "wall_s": round(wall, 3), "cost": answer["cost"]}

    # 5. A request that cannot be answered is refused, and the reader carries on.
    bad, _ = ask({"image": os.path.join(SCREENS, "no-such.png")})
    still, _ = ask({"image": image("a"), "space": "desktop"})
    report["bad_request"] = {"answer": bad, "then_still_answers": "lines" in still}

    # 6. It leaves when its input closes.
    reader.stdin.close()
    try:
        reader.wait(timeout=5)
        report["exits_when_input_closes"] = True
    except subprocess.TimeoutExpired:
        reader.kill()
        report["exits_when_input_closes"] = False
    report["total_s"] = round(time.perf_counter() - started, 1)
    print(json.dumps(report, indent=1))


if __name__ == "__main__":
    main()
