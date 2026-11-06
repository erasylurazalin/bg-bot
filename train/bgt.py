#!/usr/bin/env python3
"""bgt: start, stop and check bg-bot training from a terminal.

    bgt                     what's running, and the latest numbers
    bgt check [RUN]         the same, for RUN (default: the running or last run)
    bgt go HOURS [RUN]      train RUN for HOURS more (asks which run if left out)
    bgt stop                stop training (the last checkpoint stays)
    bgt half on|off [RUN]   cap training at about half the PC (GPU memory 8 GB,
                            8 CPU threads left free, lowest priority); restarts
                            a running run so it applies now
    bgt night on|off [RUN]  the whole GPU for training while the desktop is off
                            (train/night/bg-night): no recompute, fp16
                            observations, GPU memory 15 GB, and the largest
                            micro-batch the run's model fits under that; restarts
                            a running run so it applies now
    bgt menu                a full-screen menu that refreshes itself (what the
                            app launcher entry, train/bgt.desktop, opens)

The three strengths, full, half and night, are exclusive: turning one on
drops the other two's flags from the run's args file.

A new run copies its lineup and flags from the last run and is named rDDMM
by default (today's date, with a letter added if that name is taken). Runs
live in train/runs/; go.sh does the training, stop.sh the stopping.
"""

import csv, datetime, os, re, subprocess, sys
from pathlib import Path

TRAIN = Path(__file__).resolve().parent
RUNS = TRAIN / "runs"
GOLOG = RUNS / "go.log"
HALF = ["--micro-batch", "2048", "--vram-gb", "8"]
HALF_PREFIX = ["nice", "-n", "19", "taskset", "-c", "0-23"]
# Night: the whole GPU for training while the desktop is off. The attn blocks
# keep their activations (--no-recompute) and the rollout's observations are
# float16, which is exact: every observation is a whole number and the largest
# one in 32,768 real observations was 472, far below the 2048 float16 holds
# exactly. 15 GB of the card's 16.3 leaves about a gigabyte for the driver.
# Night does not keep room for Hyprland because bg-day puts a running run back
# to half strength before the desktop comes back.
NIGHT = ["--no-recompute", "--obs-fp16", "--vram-gb", "15"]
# Night's micro-batch, from the model shape in the run's args file. No GPU was
# reachable to measure (see docs/engine.md), so these are the largest sizes
# whose peak stays about a gigabyte under the 15 GB cap, from counting the
# activations autograd keeps for the backward pass: the big run (r1105-big, 4
# layers of d256 with ff512) 13.3 GB at 8192 and 7.8 GB at 4096, so 4096; the
# attn default (3 layers of d128 with ff256) 7.4 GB at 8192, so 8192. Any
# other shape gets 2048, half's size. The size hardly touches speed: r1105-big
# did 9.90 s per update at 2048 and 9.60 s at 8192.
NIGHT_MICRO_BATCH = {(128, 3, 4, 256): "8192", (256, 4, 8, 512): "4096"}
NIGHT_MICRO_BATCH_OTHER = "2048"
STRENGTHS = ("full", "half", "night")
# The flags a strength owns: turning one on drops all of them first.
STRENGTH_FLAGS = ("--micro-batch", "--vram-gb", "--no-recompute", "--obs-fp16")
FLAGS_WITH_VALUE = ("--micro-batch", "--vram-gb")


def model_shape(a):
    """(d, layers, heads, ff) of the attn model in args list A, with model.py's
    defaults where a flag is missing."""
    def num(flag, default):
        return int(a[a.index(flag) + 1]) if flag in a else default
    return (num("--attn-d", 128), num("--attn-layers", 3),
            num("--attn-heads", 4), num("--attn-ff", 256))


def strength_args(name, a):
    """The flags strength NAME puts in args list A: night's micro-batch follows
    the model A describes."""
    if name == "night":
        return NIGHT + ["--micro-batch", NIGHT_MICRO_BATCH.get(model_shape(a), NIGHT_MICRO_BATCH_OTHER)]
    return HALF if name == "half" else []


def pgrep(pattern):
    r = subprocess.run(["pgrep", "-af", pattern], capture_output=True, text=True)
    return [l for l in r.stdout.splitlines() if l.strip()]


def running():
    """The run name if training is going (go.sh loop or ppo.py), else None."""
    for l in pgrep(r"^\.\./\.venv/bin/python ppo.py") + pgrep(r"^(/bin/)?bash [^ ]*go\.sh [0-9]"):
        m = re.search(r"--name (\S+)", l) or re.search(r"go\.sh \S+ (\S+)", l)
        if m:
            return m.group(1)
    return None


def runs_by_time():
    """Run names with an args file, newest first."""
    files = sorted(RUNS.glob("*-args.txt"), key=lambda p: p.stat().st_mtime, reverse=True)
    return [f.name[:-len("-args.txt")] for f in files]


def last_run():
    lines = GOLOG.read_text().splitlines() if GOLOG.exists() else []
    for l in reversed(lines):
        m = re.search(r" go [\d.]+ h: (\S+) ", l)
        if m and (RUNS / f"{m.group(1)}-args.txt").exists():
            return m.group(1)
    r = runs_by_time()
    return r[0] if r else None


def suggest_name():
    base = "r" + datetime.date.today().strftime("%d%m")
    name, i = base, 0
    while (RUNS / f"{name}-args.txt").exists() or (RUNS / name).exists():
        i += 1
        name = base + "abcdefghijklmnopqrstuvwxyz"[i - 1]
    return name


def args_of(run):
    p = RUNS / f"{run}-args.txt"
    return p.read_text().split() if p.exists() else []


def strength(run):
    """The strength RUN's args file holds. Night is half's flags plus flags of
    its own, so it is tested first."""
    a = args_of(run)
    if "--no-recompute" in a or "--obs-fp16" in a:
        return "night"
    return "half" if "--vram-gb" in a else "full"


def set_strength(run, name):
    a = args_of(run)
    for flag in STRENGTH_FLAGS:
        while flag in a:
            i = a.index(flag)
            del a[i:i + (2 if flag in FLAGS_WITH_VALUE else 1)]
    a += strength_args(name, a)
    (RUNS / f"{run}-args.txt").write_text(" ".join(a) + "\n")


def until_of(run):
    """The minute go.sh stops RUN at, from the last 'go' line for it."""
    if not GOLOG.exists():
        return None
    for l in reversed(GOLOG.read_text().splitlines()):
        m = re.search(rf" go [\d.]+ h: {re.escape(run)} from \d+ to (\d+) min", l)
        if m:
            return int(m.group(1))
    return None


def latest_out(run):
    outs = sorted(RUNS.glob(f"{run}-2*.out"), key=lambda p: p.stat().st_mtime)
    return outs[-1] if outs else None


def last_update(run):
    """The newest update line of RUN and when it was written: the newest log
    that has one (right after a start the new log is still empty)."""
    outs = sorted(RUNS.glob(f"{run}-2*.out"), key=lambda p: p.stat().st_mtime, reverse=True)
    for out in outs:
        ups = [l for l in out.read_text(errors="replace").splitlines() if l.startswith("update")]
        if ups:
            return ups[-1], datetime.datetime.fromtimestamp(out.stat().st_mtime), out
    return None


def current_minute(run):
    """The run's training minute: the newest update line over all its logs (a
    fresh log has none yet), else go.log's last "start at N min"."""
    best = None
    for out in RUNS.glob(f"{run}-2*.out"):
        for l in out.read_text(errors="replace").splitlines():
            if l.startswith("update"):
                m = re.search(r"([\d.]+) min", l)
                if m and (best is None or float(m.group(1)) > best):
                    best = float(m.group(1))
    if GOLOG.exists():
        lines = GOLOG.read_text().splitlines()
        for i in range(len(lines) - 1, -1, -1):
            if re.search(rf" go [\d.]+ h: {re.escape(run)} ", lines[i]):
                for l in lines[i:]:
                    m = re.search(r"start at (\d+) min", l)
                    if m and (best is None or int(m.group(1)) > best):
                        best = float(m.group(1))
                break
    return best


def g(pat, s, default="?"):
    m = re.search(pat, s)
    return m.group(1) if m else default


def best_real(run):
    p = RUNS / run / "metrics.csv"
    if not p.exists():
        return None
    best = None
    with open(p) as f:
        for row in csv.DictReader(f):
            try:
                v = float(row["place_vs_real"])
            except (KeyError, ValueError):
                continue
            if v == v and (best is None or v < best[0]):
                best = (v, row["minutes"])
    return best


def gpu():
    r = subprocess.run(["nvidia-smi", "--query-gpu=utilization.gpu,memory.used,memory.total",
                        "--format=csv,noheader,nounits"], capture_output=True, text=True)
    try:
        u, used, tot = [x.strip() for x in r.stdout.split(",")]
        return f"GPU {u}% busy, memory {int(used) / 1024:.1f} of {int(tot) / 1024:.1f} GB"
    except ValueError:
        return "GPU: nvidia-smi gave nothing"


def check(run=None):
    live = running()
    run = run or live or last_run()
    if not run:
        print("no runs yet")
        return
    state = "RUNNING" if live == run else ("stopped" + (f" (running now: {live})" if live else ""))
    print(f"{run}: {state}, {strength(run)} strength")
    lu = last_update(run)
    up = lu[0] if lu else None
    if up:
        minute = float(g(r"([\d.]+) min", up, "0"))
        fresh = latest_out(run) != lu[2]
        print(f"  update {g(r'update\s+(\d+)', up)}, minute {minute:.0f}, {g(r'(\d+) games', up)} games"
              f"   (written {lu[1]:%H:%M}{', before the last restart; the first new line comes in a few minutes' if fresh else ''})")
        print(f"  vs 7 real {g(r'real ([\d.]+)', up)}   vs 7 standard {g(r'std ([\d.]+)', up)}   "
              f"tier 3/4/5 on turns {g(r't3/4/5 (\S+)', up)}   entropy {g(r'ent ([\d.]+)', up)}   "
              f"{g(r'(\d+) steps/s', up)} steps/s")
        b = best_real(run)
        if b:
            print(f"  best vs 7 real so far: {b[0]:.2f} (minute {float(b[1]):.0f})")
        u = until_of(run)
        if live == run and u:
            left = max(0, u - (current_minute(run) or minute))
            eta = datetime.datetime.now() + datetime.timedelta(minutes=left)
            print(f"  stops at minute {u}: about {left / 60:.1f} h left, around {eta:%a %H:%M}")
    else:
        print("  no update lines yet")
    if GOLOG.exists():
        lines = GOLOG.read_text().splitlines()
        starts = [i for i, l in enumerate(lines) if re.search(rf" go [\d.]+ h: {re.escape(run)} ", l)]
        if starts:
            since = lines[starts[-1]:]
            # stop the window at the next run's go line
            for j, l in enumerate(since[1:], 1):
                if re.search(r" go [\d.]+ h: ", l):
                    since = since[:j]
                    break
            crashes = [l for l in since if re.search(r"exit [1-9]", l)]
            print(f"  since its last go: {len(crashes)} crash(es)"
                  + (f", last: {crashes[-1].strip()}" if crashes else "")
                  + (", gave up after 5 tries" if len(crashes) >= 5 else ""))
    print(f"  {gpu()}")
    if GOLOG.exists():
        print("  go.log:")
        for l in GOLOG.read_text().splitlines()[-3:]:
            l = re.sub(r", lineup .*", "", l)  # the flags are long; bgt shows them elsewhere
            print("    " + l.strip())


def start(run, hours):
    s = strength(run)
    # night is the whole PC, so only half gets the nice/taskset prefix
    cmd = (HALF_PREFIX if s == "half" else []) + ["./go.sh", str(hours), run]
    # a new session, so closing the terminal (or the menu's window) doesn't hang up on it
    r = subprocess.run(cmd, cwd=TRAIN, capture_output=True, text=True,
                       stdin=subprocess.DEVNULL, start_new_session=True)
    if r.returncode != 0 or r.stdout.strip():
        print((r.stdout + r.stderr).strip())
    if r.returncode == 0:
        print(f"started {run} for {hours} h ({s} strength). Check with: bgt check")


def go(hours, run=None):
    live = running()
    if live:
        sys.exit(f"{live} is already running. Stop it first: bgt stop")
    try:
        h = float(hours)
        assert h > 0
    except (ValueError, AssertionError):
        sys.exit("HOURS must be a positive number, e.g. bgt go 12")
    if not run:
        last = last_run()
        if last and sys.stdin.isatty():
            ans = input(f"continue {last}? [Y/n] ").strip().lower()
            if ans in ("", "y", "yes"):
                run = last
        if not run:
            sug = suggest_name()
            ans = input(f"new run name [{sug}]: ").strip() if sys.stdin.isatty() else ""
            run = ans or sug
    if not re.fullmatch(r"[A-Za-z0-9_.-]+", run):
        sys.exit(f"bad run name: {run}")
    if not (RUNS / f"{run}-args.txt").exists() and not (RUNS / run / "ckpt.pt").exists():
        src = runs_by_time()[0] if runs_by_time() else None
        if not src:
            sys.exit("no earlier run to copy the lineup and flags from")
        for kind in ("args", "lineup"):
            f = RUNS / f"{src}-{kind}.txt"
            if f.exists():
                (RUNS / f"{run}-{kind}.txt").write_text(f.read_text())
        print(f"new run {run}: lineup and flags copied from {src} (fresh network, no checkpoint)")
    start(run, hours)


def stop():
    if not running():
        print("no training running")
        return
    stop_quiet()


def stop_quiet():
    r = subprocess.run(["./stop.sh"], cwd=TRAIN, capture_output=True, text=True)
    out = (r.stdout + r.stderr).strip()
    if out:
        print(out)


def set_run_strength(name, onoff, run=None):
    """bgt half|night on|off [RUN]: turn one strength on (and the others off),
    or off (back to full), restarting a running run with the time it had left."""
    if onoff not in ("on", "off"):
        sys.exit(f"usage: bgt {name} on|off [RUN]")
    live = running()
    run = run or live or last_run()
    if not run or not (RUNS / f"{run}-args.txt").exists():
        sys.exit(f"no args file for {run}")
    now = strength(run)
    want = name if onoff == "on" else "full"
    if now == want:
        print(f"{run} is already at {want} strength")
        return
    set_strength(run, want)
    print(f"{run} set to {want} strength")
    if live == run:
        u = until_of(run)
        minute = current_minute(run)
        left = (u - minute) / 60 if u and minute is not None else None
        stop_quiet()
        if left and left > 0.05:
            print(f"restarting with the {left:.1f} h it had left")
            start(run, round(left, 2))
        else:
            print("it was about to stop anyway; not restarted")


def captured(fn, *a):
    """Run fn and return what it printed (and its exit message) as lines."""
    import contextlib, io
    buf = io.StringIO()
    with contextlib.redirect_stdout(buf):
        try:
            fn(*a)
        except SystemExit as e:
            if e.code not in (None, 0):
                print(e.code)
    return buf.getvalue().rstrip("\n").splitlines()


def menu():
    import curses

    def ui(scr):
        curses.curs_set(0)
        curses.use_default_colors()
        for i, c in enumerate((curses.COLOR_GREEN, curses.COLOR_YELLOW, curses.COLOR_CYAN), 1):
            curses.init_pair(i, c, -1)
        curses.mousemask(curses.ALL_MOUSE_EVENTS)
        scr.timeout(5000)
        sel, msg = 0, []
        status = captured(check)

        def items():
            live = running()
            run = live or last_run()
            s = strength(run) if run else "full"
            return [("Go", "start or continue a run"),
                    ("Stop", "stop the running run" if live else "nothing is running"),
                    (f"Strength: {s}",
                     "full, half (half the PC) or night (all of it); a running run "
                     "restarts with its time left"),
                    ("Refresh", "the status refreshes itself every 5 s"),
                    ("Quit", "training keeps going after this window closes")]

        def ask(prompt, default=""):
            h, _ = scr.getmaxyx()
            curses.curs_set(1)
            curses.echo()
            scr.timeout(-1)
            scr.move(h - 2, 0)
            scr.clrtoeol()
            scr.addstr(h - 2, 1, prompt + (f" [{default}]" if default else "") + ": ",
                       curses.color_pair(3) | curses.A_BOLD)
            try:
                ans = scr.getstr().decode().strip()
            except KeyboardInterrupt:
                ans = None
            curses.noecho()
            curses.curs_set(0)
            scr.timeout(5000)
            return default if ans == "" else ans

        def do_go():
            live = running()
            if live:
                return [f"{live} is already running. Stop it first."]
            hours = ask("Hours to train (Enter on empty cancels)")
            if not hours:
                return ["cancelled"]
            last = last_run()
            run = None
            if last:
                a = ask(f"Continue {last}? (y/n)", "y")
                if a is None:
                    return ["cancelled"]
                if a.lower().startswith("y"):
                    run = last
            if not run:
                run = ask("New run name", suggest_name())
                if not run:
                    return ["cancelled"]
            return captured(go, hours, run)

        def busy(text):
            h, w = scr.getmaxyx()
            scr.move(h - 2, 0)
            scr.clrtoeol()
            scr.addstr(h - 2, 1, text[:w - 2], curses.color_pair(2) | curses.A_BOLD)
            scr.refresh()

        def act(i):
            try:
                return act_(i)
            except KeyboardInterrupt:
                raise
            except Exception as e:
                return [f"error: {type(e).__name__}: {e}"]

        def act_(i):
            name = items()[i][0]
            if name == "Go":
                return do_go()
            if name == "Stop":
                if not running():
                    return ["nothing is running"]
                a = ask(f"Stop {running()}? (y/n)", "n")
                if not (a and a.lower().startswith("y")):
                    return ["not stopped"]
                busy("stopping... (can take up to 30 s)")
                return captured(stop)
            if name.startswith("Strength"):
                run = running() or last_run()
                s = strength(run) if run else "full"
                nxt = STRENGTHS[(STRENGTHS.index(s) + 1) % len(STRENGTHS)]
                if running():
                    busy(f"restarting the run at {nxt} strength... (up to 30 s)")
                return captured(set_run_strength, nxt, "on", run)
            if name == "Refresh":
                return []
            raise KeyboardInterrupt

        while True:
            its = items()
            scr.erase()
            h, w = scr.getmaxyx()
            y = 0
            scr.addstr(y, 1, "bg-bot training", curses.A_BOLD)
            scr.addstr(y, 18, datetime.datetime.now().strftime("%H:%M:%S"), curses.A_DIM)
            y += 2
            for l in status:
                if y >= h - 9:
                    break
                attr = curses.color_pair(1) | curses.A_BOLD if "RUNNING" in l else 0
                if "crash" in l and "0 crash" not in l:
                    attr = curses.color_pair(2) | curses.A_BOLD
                scr.addstr(y, 1, l[:w - 2], attr)
                y += 1
            y += 1
            menu_top = y
            for i, (name, hint) in enumerate(its):
                line = f" {i + 1}  {name:<24}"
                scr.addstr(y, 1, line[:w - 2], curses.A_REVERSE if i == sel else 0)
                if len(line) + 3 < w:
                    scr.addstr(y, len(line) + 3, hint[:w - len(line) - 4], curses.A_DIM)
                y += 1
            y += 1
            for l in msg[-max(1, h - y - 2):]:
                if y >= h - 2:
                    break
                scr.addstr(y, 1, l[:w - 2], curses.color_pair(3))
                y += 1
            scr.addstr(h - 1, 1, "arrows/numbers/mouse pick, Enter runs, q quits"[:w - 2], curses.A_DIM)
            scr.refresh()
            try:
                k = scr.getch()
                if k == -1:
                    status = captured(check)
                    continue
                if k in (ord("q"), 27):
                    break
                if k in (curses.KEY_UP, ord("k")):
                    sel = (sel - 1) % len(its)
                elif k in (curses.KEY_DOWN, ord("j"), 9):
                    sel = (sel + 1) % len(its)
                elif ord("1") <= k < ord("1") + len(its):
                    sel = k - ord("1")
                    msg = act(sel)
                elif k in (10, 13, curses.KEY_ENTER):
                    msg = act(sel)
                elif k == curses.KEY_MOUSE:
                    _, _, my, _, bstate = curses.getmouse()
                    if menu_top <= my < menu_top + len(its) and bstate & (curses.BUTTON1_CLICKED | curses.BUTTON1_PRESSED):
                        sel = my - menu_top
                        msg = act(sel)
                status = captured(check)
            except KeyboardInterrupt:
                break
            except curses.error:
                pass

    curses.wrapper(ui)


def main(argv):
    if not argv or argv[0] in ("check", "status"):
        return check(argv[1] if len(argv) > 1 else None)
    cmd, rest = argv[0], argv[1:]
    if cmd == "go" and rest:
        return go(*rest[:2])
    if cmd == "stop":
        return stop()
    if cmd in ("half", "night") and rest:
        return set_run_strength(cmd, *rest[:2])
    if cmd in ("menu", "--menu"):
        return menu()
    print(__doc__.strip())


if __name__ == "__main__":
    main(sys.argv[1:])
