"""Policy and value network.

Each card slot (own shop minions, shop spell, hand, board, discover, and each
opponent's last seen board) goes through the same small encoder: a learned
embedding of the card and of the spell it was taught (Magicfin), plus its
stats, tribes, keywords and counters. The slot encodings, in their fixed
order, are concatenated with the scalars (with the last spell cast and the
card a pending choice comes from embedded the same way) and go through an
MLP to the action logits and a value.
"""

import torch
from torch import nn

import bgpy

SF = bgpy.SLOT_FEATURES
OWN_SLOTS = bgpy.OWN_SLOTS
OPP = bgpy.PLAYERS - 1
OPP_BLOCK = bgpy.OPP_BLOCK
OWN_END = bgpy.OWN_END

# Card ids, embedded rather than scaled.
SCALAR_IDS = [bgpy.SCALAR_LAST_SPELL, bgpy.SCALAR_PENDING_CARD]
SLOT_IDS = [bgpy.SLOT_CARD_ID, bgpy.SLOT_TAUGHT_SPELL]
# Stats can run into the hundreds late (Holy Vanguard, Tyrael's 50/50), so
# these are log-scaled after the division.
SLOT_LOG = [bgpy.SLOT_ATTACK, bgpy.SLOT_HEALTH, bgpy.SLOT_FIGHT_ATTACK, bgpy.SLOT_FIGHT_HEALTH]


def _scales(n, prefix, table):
    """Rough scales so inputs land near 0..1: 1 unless `table` names the
    constant (bgpy.<prefix><name>), with ranges for multi-value blocks."""
    s = [1.0] * n
    for name, v in table.items():
        if isinstance(name, tuple):
            start, count = name
            for i in range(start, start + count):
                s[i] = v
        else:
            s[getattr(bgpy, prefix + name)] = v
    return s


SCALAR_SCALE = _scales(bgpy.SCALARS, "SCALAR_", {
    "GOLD": 10, "TIER": 6, "UPGRADE_COST": 10, "HEALTH": 30, "TURN": 20,
    "ACTIONS_LEFT": 60, "ALIVE": 8, "GOLD_NEXT_TURN": 5, "EXTRA_MAX_GOLD": 5,
    "FREE_ROLLS": 5, "HEALTH_REFRESHES_LEFT": 4, "SPELL_DISCOUNT": 3,
    "SHOP_SPELL_COST": 5, "SPELL_ATTACK": 10, "SPELL_HEALTH": 10,
    "VOLUMIZER_ATTACK": 20, "VOLUMIZER_HEALTH": 20, "BEETLE_ATTACK": 50, "BEETLE_HEALTH": 50,
    "BOON_OF_BEETLES": 4, "BROOD_OF_NOZDORMU": 2, "UPPER_HAND": 2, "PATIENT_SCOUT_TIER": 6,
    "TASTY_LOBSTER": 10, "BEAST_BONUS_ATTACK": 14, "BEAST_BONUS_HEALTH": 14,
    "DEATHRATTLES": 50, "FODDER_NEXT": 3, "FODDER_QUEUED": 5, "FODDER_REFRESHES": 5,
    "MAGICFIN_LEFT": 2, "OVERCONFIDENCE_WIN": 3, "OVERCONFIDENCE_TIE": 3,
    "WINNERS_BREAD_ATTACK": 10, "WINNERS_BREAD_HEALTH": 10,
    "NEXT_TURN_BUFF_ATTACK": 10, "NEXT_TURN_BUFF_HEALTH": 10,
    (bgpy.SCALAR_TAVERN_BUFFS, bgpy.TAVERN_BUFF_FILTERS * 4): 10,
    "PENDING_SOURCE": 7, "PENDING_RUNS": 2, "PENDING_ATTACK": 20, "PENDING_HEALTH": 20,
    "SHOP_CONSUME_LEFT": 3, "DISCOVER_QUEUE": 3,
    **{(bgpy.SCALAR_CHOOSE_ONE + j * bgpy.CHOOSE_FEATURES + 4, 2): 10 for j in range(2)},
})
OPP_SCALE = _scales(bgpy.OPP_SCALARS, "OPP_", {
    "HEALTH": 30, "TIER": 6, "TURNS_SINCE": 10, "PLACEMENT": 8, (bgpy.OPP_TRIBES, 5): 7,
})
SLOT_SCALE = _scales(SF, "SLOT_", {
    "ATTACK": 10, "HEALTH": 10, "FIGHT_ATTACK": 10, "FIGHT_HEALTH": 10, "TIER": 6,
    "MAGNETIZATIONS": 10, "VOLUMIZERS": 10, "IMPROVEMENTS": 10, "EVENTS_LEFT": 4,
    "WINNERS_BREAD": 2, "ACTIVATE_COST": 2, "EXTRA_MAGNETIZE": 2, "COST": 3,
})

assert len(SCALAR_SCALE) == bgpy.SCALARS
assert len(OPP_SCALE) == bgpy.OPP_SCALARS
assert len(SLOT_SCALE) == SF


class Net(nn.Module):
    def __init__(self, n_cards=bgpy.N_CARDS, emb=32, slot=64, hidden=512):
        super().__init__()
        self.card = nn.Embedding(n_cards + 1, emb)  # 0 is an empty slot or no card
        feats = [i for i in range(SF) if i not in SLOT_IDS]
        self.slot = nn.Sequential(nn.Linear(len(SLOT_IDS) * emb + len(feats), slot), nn.ReLU())
        n_slots = bgpy.TOTAL_SLOTS
        n_in = (n_slots * slot + bgpy.SCALARS + len(SCALAR_IDS) * emb
                + OPP * bgpy.OPP_SCALARS)
        self.trunk = nn.Sequential(
            nn.Linear(n_in, hidden), nn.ReLU(),
            nn.Linear(hidden, hidden), nn.ReLU(),
        )
        self.policy = nn.Linear(hidden, bgpy.N_ACTIONS)
        self.value = nn.Linear(hidden, 1)
        nn.init.orthogonal_(self.policy.weight, 0.01)
        with torch.no_grad():
            self.policy.bias.copy_(kind_prior())
        keep = torch.ones(bgpy.SCALARS)
        keep[SCALAR_IDS] = 0.0
        self.register_buffer("s_scale", torch.tensor(SCALAR_SCALE, dtype=torch.float32))
        self.register_buffer("s_keep", keep)
        self.register_buffer("o_scale", torch.tensor(OPP_SCALE, dtype=torch.float32))
        self.register_buffer("m_scale", torch.tensor(SLOT_SCALE, dtype=torch.float32)[feats])
        self.register_buffer("feats", torch.tensor(feats, dtype=torch.long))
        self.register_buffer("log_cols", torch.tensor([feats.index(i) for i in SLOT_LOG]))

    def forward(self, obs):
        b = obs.shape[0]
        raw = obs[:, :bgpy.SCALARS]
        scalars = raw / self.s_scale * self.s_keep
        s_cards = self.card(raw[:, SCALAR_IDS].long()).reshape(b, -1)
        own = obs[:, bgpy.SCALARS:OWN_END].reshape(b, OWN_SLOTS, SF)
        opp = obs[:, OWN_END:].reshape(b, OPP, OPP_BLOCK)
        opp_scalars = (opp[:, :, :bgpy.OPP_SCALARS] / self.o_scale).reshape(b, -1)
        opp_slots = opp[:, :, bgpy.OPP_SCALARS:].reshape(b, OPP * bgpy.BOARD, SF)
        slots = torch.cat([own, opp_slots], 1)
        n = slots.shape[1]
        cards = self.card(slots[:, :, SLOT_IDS].long()).reshape(b, n, -1)
        feats = slots[:, :, self.feats] / self.m_scale
        feats[:, :, self.log_cols] = torch.log1p(feats[:, :, self.log_cols].clamp(min=0))
        enc = self.slot(torch.cat([cards, feats], 2)).reshape(b, -1)
        h = self.trunk(torch.cat([enc, scalars, s_cards, opp_scalars], 1))
        return self.policy(h), self.value(h).squeeze(1)


KIND_SIZES = [n for _, n in bgpy.ACTION_KINDS]
assert sum(KIND_SIZES) == bgpy.N_ACTIONS


def kind_prior():
    """Starting logits that make every kind of action about equally likely,
    whatever its number of arguments. Uniform over all actions would almost
    never end a turn, since 42 of them are moves and 80 are plays."""
    return torch.cat([torch.full((n,), -float(torch.tensor(n).log())) for n in KIND_SIZES])


def masked(logits, mask):
    return logits.masked_fill(~mask, -1e9)
