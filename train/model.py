"""Policy and value network.

Each card slot (own shop minions, shop spell, hand, board, discover, and each
opponent's last seen board) goes through the same small encoder: a learned
embedding of the card plus its stats, tribe, keywords and counters. The slot
encodings, in their fixed order, are concatenated with the scalars and go
through an MLP to the action logits and a value.
"""

import torch
from torch import nn

import bgpy

SF = bgpy.SLOT_FEATURES
OWN_SLOTS = bgpy.OWN_SLOTS
OPP = bgpy.PLAYERS - 1
OPP_BLOCK = bgpy.OPP_BLOCK
OWN_END = bgpy.OWN_END

# Rough scales so inputs land near 0..1. Order follows bgcore/src/obs.rs.
SCALAR_SCALE = [
    10, 6, 10, 30, 20, 1, 1, 7, 1, 60, 8, 1,
    20, 20, 50, 50, 6, 5, 5, 5, 3, 10, 10, 4, 2, 2, bgpy.N_CARDS, 1, 1, 1,
]
OPP_SCALE = [1, 30, 6, 1, 10, 1]
# present, card (embedded, not scaled), is_spell, attack, health, golden, tier,
# 3 tribe flags, 6 keywords, magnetizations, volumizers, improvements, from_pool, winners_bread
SLOT_SCALE = [
    1, 1, 1, 20, 20, 1, 6,
    1, 1, 1,
    1, 1, 1, 1, 1, 1,
    10, 10, 10, 1, 2,
]

assert len(SCALAR_SCALE) == bgpy.SCALARS
assert len(OPP_SCALE) == bgpy.OPP_SCALARS
assert len(SLOT_SCALE) == SF


class Net(nn.Module):
    def __init__(self, n_cards=bgpy.N_CARDS, emb=32, slot=64, hidden=512):
        super().__init__()
        self.card = nn.Embedding(n_cards + 1, emb)  # 0 is an empty slot
        self.slot = nn.Sequential(nn.Linear(emb + SF - 1, slot), nn.ReLU())
        n_slots = bgpy.TOTAL_SLOTS
        n_in = n_slots * slot + bgpy.SCALARS + OPP * bgpy.OPP_SCALARS
        self.trunk = nn.Sequential(
            nn.Linear(n_in, hidden), nn.ReLU(),
            nn.Linear(hidden, hidden), nn.ReLU(),
        )
        self.policy = nn.Linear(hidden, bgpy.N_ACTIONS)
        self.value = nn.Linear(hidden, 1)
        nn.init.orthogonal_(self.policy.weight, 0.01)
        with torch.no_grad():
            self.policy.bias.copy_(kind_prior())
        self.register_buffer("s_scale", torch.tensor(SCALAR_SCALE, dtype=torch.float32))
        self.register_buffer("o_scale", torch.tensor(OPP_SCALE, dtype=torch.float32))
        self.register_buffer("m_scale", torch.tensor(SLOT_SCALE, dtype=torch.float32))

    def forward(self, obs):
        b = obs.shape[0]
        scalars = obs[:, :bgpy.SCALARS] / self.s_scale
        own = obs[:, bgpy.SCALARS:OWN_END].reshape(b, OWN_SLOTS, SF)
        opp = obs[:, OWN_END:].reshape(b, OPP, OPP_BLOCK)
        opp_scalars = (opp[:, :, :bgpy.OPP_SCALARS] / self.o_scale).reshape(b, -1)
        opp_slots = opp[:, :, bgpy.OPP_SCALARS:].reshape(b, OPP * bgpy.BOARD, SF)
        slots = torch.cat([own, opp_slots], 1) / self.m_scale
        cards = self.card(slots[:, :, bgpy.SLOT_CARD_ID].long())
        feats = torch.cat([slots[:, :, :bgpy.SLOT_CARD_ID], slots[:, :, bgpy.SLOT_CARD_ID + 1:]], 2)
        enc = self.slot(torch.cat([cards, feats], 2)).reshape(b, -1)
        h = self.trunk(torch.cat([enc, scalars, opp_scalars], 1))
        return self.policy(h), self.value(h).squeeze(1)


KIND_SIZES = [n for _, n in bgpy.ACTION_KINDS]
assert sum(KIND_SIZES) == bgpy.N_ACTIONS


def kind_prior():
    """Starting logits that make every kind of action about equally likely,
    whatever its number of arguments. Uniform over all 150 actions would
    almost never end a turn, since 42 of them are moves and 80 are plays."""
    return torch.cat([torch.full((n,), -float(torch.tensor(n).log())) for n in KIND_SIZES])


def masked(logits, mask):
    return logits.masked_fill(~mask, -1e9)
