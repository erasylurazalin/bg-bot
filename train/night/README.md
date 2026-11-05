# Night mode: desktop off, training on

Two scripts. `bg-night` closes the desktop, keeps training going at full
("night") strength and leaves a tmux session to watch it from another machine.
`bg-day` gives the desktop back. Both live here, next to `train/bgt.py`.

## Start

In a terminal on the desktop (inside Hyprland):

    train/night/bg-night 8              # train 8 more hours
    train/night/bg-night 8 r1105        # the same, for run r1105
    train/night/bg-night --dry-run 8    # print every step, change nothing

What it does, in order:

1. `bgt night on` when a run is going: it is restarted at night strength with
   the time it had left, so HOURS is ignored then. Otherwise
   `bgt night on RUN` (RUN defaults to the last run) and `bgt go HOURS RUN`.
   Training runs in a systemd user service, outside the desktop session, so it
   does not stop when the desktop does.
2. tmux session `bg`, also outside the session: window `bgt` is `bgt menu`,
   window `shell` is a shell in the repo.
3. Writes `/run/user/1000/bg-night`. While that file exists, `~/.bash_profile`
   gives tty1 the tmux session instead of starting Hyprland again.
4. Closes every window gracefully (the close-window dispatcher, one client at
   a time), waits up to 20 s, prints whatever is still open, stops the user
   services that use the GPU (`llama`, Sunshine) if they were active, and asks
   Hyprland to exit.

`bg-boards.timer` and `bg-twitch.service` are left alone: they collect logs.

The tmux session and training outlive the desktop, so you can keep using the
PC over ssh the whole time.

## Desktop off without training

`bg-night --no-train` does the same desktop-off and tmux steps but starts no
training, for running an agent (or anything else) on a fully idle PC from the
tmux shell. It refuses if a run is already going, since closing the desktop
would kill it. `bg-day` ends it the same way.

## Watch it from another machine

    ssh era-arch
    tmux attach -t bg

`bgt menu` in the first window and `bgt check` in the second are enough for
most nights. The log of what night mode did is `train/runs/night.log`.

## End night mode

On the PC console there is a tmux session with a shell in it, so:

    train/night/bg-day              # inside tmux
    # then detach with Ctrl-b d

Over ssh it is the same command:

    ssh era-arch
    train/night/bg-day

`bg-day` switches a running run back to half strength (`bgt half on`, which
restarts it with the time it has left), starts the services `bg-night`
stopped, and removes the flag. The desktop itself only comes back on the
console: after detaching, the login shell on tty1 ends, getty starts a new
one, and since the flag is gone `~/.bash_profile` starts Hyprland (with
hyprlock asking for the password). Over ssh you still have to be at the PC
for that step, or log in on the console.

`bg-day --dry-run` prints every step and changes nothing.

## Reboot

The flag lives in `/run/user/1000`, which is emptied on every boot, so a
reboot gives the normal desktop. Training does not come back by itself: start
it again with `bgt go HOURS RUN`, or run `bg-night` again.

## If something looks wrong

    bgt check                       # is training running, and how far it is
    tail train/runs/go.log          # every start, crash and stop
    systemctl --user list-units 'bg-*'   # the units night mode created
    tmux ls                         # is the session still there

- No training after a restart: starting a unit of its own is the fix, so look
  at `train/runs/go.log` first; `bgt go 8 RUN` from any shell works too.
- No tmux session over ssh: `tmux new -s bg` and run `bgt menu` in it.
- To stop what night mode started: `bgt stop` (training) and
  `tmux kill-session -t bg`, and `systemctl --user stop` the `bg-*` units if
  you want them gone.

## Requires

- `bgt night on|off` (the night strength, added next to `bgt half on|off`).
  Without it `bg-night` says so and starts training at whatever strength the
  run's args file holds.
- The `~/.bash_profile` block that skips Hyprland while
  `/run/user/1000/bg-night` exists and attaches the console to tmux instead.
  Without that block the console simply starts Hyprland again: training and
  the session keep running, you just get the desktop back.
- systemd user units for training and tmux (`systemd-run --user`), `jq`,
  `tmux`, `hyprctl`.
