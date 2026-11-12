"""The scam games in docs/notes/mining/scam.md.

Selected by reading every final board: three or more pure scam slots
(Venomous/Poisonous, Leeroy, Heroic Underdog, Deadly Spore or the
Sneed/Titus package) and no scaled comp engine left. A single Leeroy or
Deadly Spore beside a big Felboar, Glambot or Aberration board is tech, not
scam, and stays in its comp.
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import lib

SCAM_GAMES = [
    ('Hearthstone_2026_10_26_00_48_15', 2, 9),
    ('Hearthstone_2026_10_26_00_48_15', 5, 11),
    ('Hearthstone_2026_11_05_15_51_51', 2, 13),
]


def is_scam(card):
    if card.get('name') in {'Leeroy the Reckless', 'Heroic Underdog',
                            "Sneed's New Shredder", 'Titus Rivendare'}:
        return True
    return bool({'POISONOUS', 'VENOMOUS'} & set(card.get('keywords') or []))


def main():
    games = {(g['source'], g['game']): g for g in lib.load_games()}
    for key in SCAM_GAMES:
        g = games[key[:2]]
        src, num, switch = key
        print(f'== {src}-g{num} place {g["place"]}')
        for t in g['turns']:
            if t['turn'] in (switch - 2, switch - 1):
                print(f'  board before turn {t["turn"] + 1}:', ', '.join(
                    f"{c['name']} {c['attack']}/{c['health']}" for c in lib.last_turn_board(t)))
            if t['turn'] == switch:
                print('  switch board:', ', '.join(
                    c['name'] for c in lib.last_turn_board(t)))
        print('  final:', ', '.join(c['name'] for c in lib.final_board(g)))
        print('  final scam slots:', ', '.join(
            c['name'] for c in lib.final_board(g) if is_scam(c)))


if __name__ == '__main__':
    main()
