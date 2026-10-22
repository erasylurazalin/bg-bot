"""Policy and value network.

Each minion slot (own shop, hand, board, discover, and each opponent's last
seen board) goes through the same small encoder: a learned embedding of the
card plus its stats and keywords. The slot encodings, in their fixed order,
are concatenated with the scalars and go through an MLP to the action logits
and a value.
"""

import torch
from torch import nn

import bgpy

MF = bgpy.MINION_FEATURES
OWN_SLOTS = bgpy.SHOP + bgpy.HAND + bgpy.BOARD + bgpy.DISCOVER
OPP = bgpy.PLAYERS - 1
OPP_BLOCK = bgpy.OPP_SCALARS + bgpy.BOARD * MF
OWN_END = bgpy.SCALARS + OWN_SLOTS * MF

# Rough scales so inputs land near 0..1. Order follows bgcore/src/obs.rs.
SCALAR_SCALE = [10, 6, 10, 30, 20, 1, 1, 7, 1, 60, 8, 1]
OPP_SCALE = [1, 30, 6, 1, 10, 1]
# present, card (embedded, not scaled), attack, health, golden, tier, 5 keywords
MINION_SCALE = [1, 1, 10, 10, 1, 6, 1, 1, 1, 1, 1]

assert len(SCALAR_SCALE) == bgpy.SCALARS
assert len(OPP_SCALE) == bgpy.OPP_SCALARS
assert len(MINION_SCALE) == MF


class Net(nn.Module):
    def __init__(self, n_cards, emb=32, slot=64, hidden=512):
        super().__init__()
        self.card = nn.Embedding(n_cards + 1, emb)  # 0 is an empty slot
        self.slot = nn.Sequential(nn.Linear(emb + MF - 1, slot), nn.ReLU())
        n_slots = OWN_SLOTS + OPP * bgpy.BOARD
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
        self.register_buffer("m_scale", torch.tensor(MINION_SCALE, dtype=torch.float32))

    def forward(self, obs):
        b = obs.shape[0]
        scalars = obs[:, :bgpy.SCALARS] / self.s_scale
        own = obs[:, bgpy.SCALARS:OWN_END].reshape(b, OWN_SLOTS, MF)
        opp = obs[:, OWN_END:].reshape(b, OPP, OPP_BLOCK)
        opp_scalars = (opp[:, :, :bgpy.OPP_SCALARS] / self.o_scale).reshape(b, -1)
        opp_slots = opp[:, :, bgpy.OPP_SCALARS:].reshape(b, OPP * bgpy.BOARD, MF)
        slots = torch.cat([own, opp_slots], 1) / self.m_scale
        cards = self.card(slots[:, :, 1].long())
        feats = torch.cat([slots[:, :, :1], slots[:, :, 2:]], 2)
        enc = self.slot(torch.cat([cards, feats], 2)).reshape(b, -1)
        h = self.trunk(torch.cat([enc, scalars, opp_scalars], 1))
        return self.policy(h), self.value(h).squeeze(1)


# Sizes of the 10 action kinds, in the order of bgcore/src/action.rs.
KIND_SIZES = [bgpy.SHOP, bgpy.BOARD, bgpy.HAND * (bgpy.BOARD + 1), bgpy.BOARD, bgpy.DISCOVER,
              bgpy.BOARD * (bgpy.BOARD - 1), 1, 1, 1, 1]
assert sum(KIND_SIZES) == bgpy.N_ACTIONS


def kind_prior():
    """Starting logits that make every kind of action about equally likely,
    whatever its number of arguments. Uniform over all 149 actions would
    almost never end a turn, since 42 of them are moves and 80 are plays."""
    return torch.cat([torch.full((n,), -float(torch.tensor(n).log())) for n in KIND_SIZES])


def masked(logits, mask):
    return logits.masked_fill(~mask, -1e9)
