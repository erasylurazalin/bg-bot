"""Load Era's action/real-board logs for the mining reports.

No network, no engine. The action files carry every shop step with the state
after it, so a turn can be walked from its start state to its end.
"""
import glob
import json
import os
import tomllib

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))
ACTIONS = os.path.join(ROOT, 'boards', 'actions')
REAL = os.path.join(ROOT, 'boards', 'real')
CARDS_TOML = os.path.join(ROOT, 'cards.toml')
HSJSON = os.path.join(ROOT, 'ref', 'hsjson-cards.json')


def load_games(include_unfinished=False):
    games = []
    for path in sorted(glob.glob(os.path.join(ACTIONS, '*.json'))):
        d = json.load(open(path))
        real_path = os.path.join(REAL, '%s-g%d.json' % (d['source'], d['game']))
        r = json.load(open(real_path)) if os.path.exists(real_path) else {}
        place = r.get('place') or 0
        if place == 0 and not include_unfinished:
            continue
        d['place'] = place
        d['path'] = path
        d['real'] = r
        games.append(d)
    return games


def load_cards():
    """name -> the card's own data, from cards.toml plus HearthstoneJSON."""
    by_name = {}
    with open(CARDS_TOML, 'rb') as f:
        toml = tomllib.load(f)
    for kind in ('minion', 'token', 'spell', 'trinket'):
        for c in toml.get(kind, []):
            e = dict(c)
            e['kind'] = kind
            by_name.setdefault(c['name'], e)
    hs = json.load(open(HSJSON))
    by_id = {}
    for c in hs:
        cid = c.get('id')
        if not cid:
            continue
        by_id[cid] = c
        if c.get('isBattlegroundsPoolMinion') and c.get('name'):
            e = by_name.setdefault(c['name'], {})
            e.setdefault('tech_level', c.get('techLevel'))
            e.setdefault('races', c.get('races') or [])
            e.setdefault('type', c.get('type'))
            e.setdefault('text', c.get('text') or '')
            e.setdefault('hs_id', cid)
    return by_name, by_id


def meta(name, by_name):
    e = by_name.get(name)
    return e or {}


def tier_of(name, by_name):
    e = meta(name, by_name)
    t = e.get('tier', e.get('tech_level'))
    return t


def tribe_of(name, by_name):
    e = meta(name, by_name)
    return e.get('tribe') or ','.join(x.lower() for x in (e.get('races') or []))


def cost_of(card):
    """The gold price a Tavern card shows, or the minion rule price."""
    if card.get('cost') is not None:
        return card['cost']
    return 3


def cards_in(state, where):
    return state.get(where) or []


def names(state, where):
    return [c.get('name') for c in cards_in(state, where)]


def all_turn_cards(turn):
    """The Tavern cards seen during the turn, in state order."""
    seen = []
    def add(state):
        for c in state.get('tavern') or []:
            seen.append(c)
    add(turn['start'])
    for a in turn['actions']:
        if a.get('after'):
            add(a['after'])
    return seen


def action_cards(turn, step):
    return [a.get('card', {}).get('name') for a in turn['actions'] if a['step'] == step]


def step_count(turn, step):
    return sum(1 for a in turn['actions'] if a['step'] == step)


def final_board(game):
    if not game['turns']:
        return []
    end = game['turns'][-1].get('end')
    if end:
        return end.get('board') or []
    # fall back to the last step's after
    for a in reversed(game['turns'][-1]['actions']):
        if a.get('after'):
            return a['after'].get('board') or []
    return []


def last_turn_board(turn):
    if turn.get('end'):
        return turn['end'].get('board') or []
    for a in reversed(turn['actions']):
        if a.get('after'):
            return a['after'].get('board') or []
    return turn['start'].get('board') or []


def board_names(board):
    return [c.get('name') for c in board or []]


def level_turns(game):
    """First turn at each tier after the game start, from the turn starts."""
    out = {}
    for t in game['turns']:
        tier = t['start'].get('tier')
        if tier and tier not in out:
            out[tier] = t['turn']
    return out


def has_name(board, name):
    return any(c.get('name') == name for c in board or [])
