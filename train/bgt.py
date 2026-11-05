#!/usr/bin/env python3
"""bgt: start, stop and check bg-bot training from a terminal.

    bgt                     what's running, and the latest numbers
    bgt check [RUN]         the same, for RUN (default: the running or last run)
    bgt go HOURS [RUN]      train RUN for HOURS more (asks which run if left out)
    bgt stop                stop training (the last checkpoint stays)
    bgt half on|off [RUN]   cap training at about half the PC (GPU memory 8 GB,
                            8 CPU threads left free, lowest priority); restarts
                            a running run so it applies now
    bgt menu                a full-screen menu that refreshes itself (what the
                            app launcher entry, train/bgt.desktop, opens)

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


def is_half(run):
    return "--vram-gb" in args_of(run)


def set_half(run, on):
    a = args_of(run)
    for flag in ("--micro-batch", "--vram-gb"):
        while flag in a:
            i = a.index(flag)
            del a[i:i + 2]
    if on:
        a += HALF
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
    out = latest_out(run)
    if not out:
        return None
    ups = [l for l in out.read_text(errors="replace").splitlines() if l.startswith("update")]
    return ups[-1] if ups else None


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
    print(f"{run}: {state}, {'half strength' if is_half(run) else 'full strength'}")
    up = last_update(run)
    if up:
        minute = float(g(r"([\d.]+) min", up, "0"))
        print(f"  update {g(r'update\s+(\d+)', up)}, minute {minute:.0f}, {g(r'(\d+) games', up)} games")
        print(f"  vs 7 real {g(r'real ([\d.]+)', up)}   vs 7 standard {g(r'std ([\d.]+)', up)}   "
              f"tier 3/4/5 on turns {g(r't3/4/5 (\S+)', up)}   entropy {g(r'ent ([\d.]+)', up)}   "
              f"{g(r'(\d+) steps/s', up)} steps/s")
        b = best_real(run)
        if b:
            print(f"  best vs 7 real so far: {b[0]:.2f} (minute {float(b[1]):.0f})")
        u = until_of(run)
        if live == run and u:
            left = max(0, u - minute)
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


def start(run, hours):
    cmd = (HALF_PREFIX if is_half(run) else []) + ["./go.sh", str(hours), run]
    r = subprocess.run(cmd, cwd=TRAIN, capture_output=True, text=True)
    if r.returncode != 0 or r.stdout.strip():
        print((r.stdout + r.stderr).strip())
    if r.returncode == 0:
        print(f"started {run} for {hours} h ({'half' if is_half(run) else 'full'} strength). "
              f"Check with: bgt check")


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
    subprocess.run(["./stop.sh"], cwd=TRAIN)


def half(onoff, run=None):
    if onoff not in ("on", "off"):
        sys.exit("usage: bgt half on|off [RUN]")
    live = running()
    run = run or live or last_run()
    if not run or not (RUNS / f"{run}-args.txt").exists():
        sys.exit(f"no args file for {run}")
    on = onoff == "on"
    if is_half(run) == on:
        print(f"{run} is already at {'half' if on else 'full'} strength")
        return
    set_half(run, on)
    print(f"{run} set to {'half' if on else 'full'} strength")
    if live == run:
        up = last_update(run)
        u = until_of(run)
        minute = float(g(r"([\d.]+) min", up or "", "0"))
        left = (u - minute) / 60 if u else None
        subprocess.run(["./stop.sh"], cwd=TRAIN)
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
            h = is_half(run) if run else False
            return [("Go", "start or continue a run"),
                    ("Stop", "stop the running run" if live else "nothing is running"),
                    (f"Half strength: {'ON' if h else 'off'}",
                     "toggle; a running run restarts with its time left"),
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

        def act(i):
            name = items()[i][0]
            if name == "Go":
                return do_go()
            if name == "Stop":
                if not running():
                    return ["nothing is running"]
                a = ask(f"Stop {running()}? (y/n)", "n")
                return captured(stop) if a and a.lower().startswith("y") else ["not stopped"]
            if name.startswith("Half"):
                run = running() or last_run()
                return captured(half, "off" if is_half(run) else "on", run)
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
    if cmd == "half" and rest:
        return half(*rest[:2])
    if cmd in ("menu", "--menu"):
        return menu()
    print(__doc__.strip())


if __name__ == "__main__":
    main(sys.argv[1:])
