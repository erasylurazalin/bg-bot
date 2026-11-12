"""Counts behind docs/notes/mining/beetles.md."""
import collections
import os
import statistics
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import lib

CORE = {'Buzzing Vermin', 'Forest Rover', 'Turquoise Skitterer', 'Ravaging Scorpid', 'Beetle'}


def core_on_board(turn):
    return [c['name'] for c in lib.last_turn_board(turn) if c['name'] in CORE]


def is_beetle_game(g):
    return sum(1 for t in g['turns'] if len(core_on_board(t)) >= 2) >= 3


def first_commit(g):
    for t in g['turns']:
        if len(core_on_board(t)) >= 2:
            return t['turn']
    return None


def first_level(g, tier):
    for t in g['turns']:
        for a in t['actions']:
            if a['step'] == 'level' and a.get('tier_after') == tier:
                return t['turn']
    return None


def main():
    games = [g for g in lib.load_games() if is_beetle_game(g)]
    print(f'beetle games: {len(games)}')
    for g in games:
        print(f"  {g['source']}-g{g['game']} place {g['place']} commit {first_commit(g)}")
    print('\ncore cards in games/turns (end boards with >=2 core), top4 vs bottom4:')
    gp = collections.Counter()
    gt = collections.Counter()
    for g in games:
        grp = 'top' if g['place'] <= 4 else 'bottom'
        seen = set()
        for t in g['turns']:
            board = lib.last_turn_board(t)
            if sum(1 for c in board if c['name'] in CORE) >= 2:
                for c in board:
                    if c['name'] in CORE:
                        gt[(grp, c['name'])] += 1
                        seen.add(c['name'])
        for n in seen:
            gp[(grp, n)] += 1
    names = {n for _, n in gp}
    for n in sorted(names, key=lambda x: -(gp[('top', x)] + gp[('bottom', x)])):
        print(f'  {n:22s} top {gp[("top", n)]}/4 games {gt[("top", n)]:3d} turns | '
              f'bottom {gp[("bottom", n)]}/1 games {gt[("bottom", n)]:3d} turns')
    print('\nlevel action turn by tier:')
    for tier in range(2, 7):
        vals = [first_level(g, tier) for g in games if first_level(g, tier)]
        print(f'  tier {tier}: {sorted(vals)} median {statistics.median(vals)}')
    print('\nearly cards before commit (game count / turn count):')
    gp2 = collections.Counter()
    gt2 = collections.Counter()
    for g in games:
        commit = first_commit(g)
        seen = set()
        for t in g['turns']:
            if t['turn'] >= commit:
                continue
            for c in lib.last_turn_board(t):
                gt2[c['name']] += 1
                seen.add(c['name'])
        for n in seen:
            gp2[n] += 1
    for n, c in gp2.most_common(15):
        print(f'  {n:26s} games {c}/5 turns {gt2[n]}')
    print('\nfinal boards:')
    for g in games:
        print(f"  {g['source']}-g{g['game']} place {g['place']}: "
              + ', '.join(c['name'] for c in lib.final_board(g)))


if __name__ == '__main__':
    main()
