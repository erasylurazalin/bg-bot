"""Policy and value network.

Each card slot (own shop minions, shop spell, hand, board, discover, and each
opponent's last seen board) goes through the same small encoder: a learned
embedding of the card and of the spell it was taught (Magicfin), plus its
stats, tribes, keywords and counters. The slot encodings, in their fixed
order, are concatenated with the scalars (with the last spell cast and the
card a pending choice comes from embedded the same way) and go through an
MLP to the action logits and a value. That is `Net`, the default ("mlp").

Two alternatives share one slot encoder (`SlotInputs`), which turns each slot
into a token with a learned zone and position embedding:
- `AttnNet` ("attn"): a small pre-norm transformer over the slot tokens, a
  token for my own scalars (the summary) and one per opponent.
- `PoolNet` ("pool"): mean and max of the tokens per zone, into an MLP.
Both score per-slot actions from the token of the slot they act on
(`ActionHead`) and global actions from the summary.
"""

import math

import torch
import torch.nn.functional as F
from torch import nn
from torch.utils.checkpoint import checkpoint

import bgpy

SF = bgpy.SLOT_FEATURES
OWN_SLOTS = bgpy.OWN_SLOTS
OPP = bgpy.PLAYERS - 1
OPP_BLOCK = bgpy.OPP_BLOCK
OWN_END = bgpy.OWN_END

# A hero id and a power id share one embedding table (`HeroDb::powers` holds
# more entries than there are heroes: the powers a hero only reaches later).
N_HERO_IDS = max(bgpy.N_HEROES, bgpy.N_POWERS)
# Card ids, embedded rather than scaled.
SCALAR_IDS = [bgpy.SCALAR_LAST_SPELL, bgpy.SCALAR_PENDING_CARD]
# Hero and power ids, embedded with their own table: own hero, the power's
# hero (the first power and the second one, both indices into the power
# table), the powers an open Hero Power Discover or chooser offers (so a pick
# is made from what it can see), and the hero offer slots during the pick.
HERO_IDS = [bgpy.SCALAR_HERO, bgpy.SCALAR_POWER_HERO, bgpy.SCALAR_POWER2_HERO] + list(
    range(bgpy.SCALAR_POWER_OFFER, bgpy.SCALAR_POWER_OFFER + bgpy.CHOICES)) + list(
    range(bgpy.SCALAR_HERO_OFFER, bgpy.SCALAR_HERO_OFFER + bgpy.DISCOVER))
# Trinket ids, embedded with their own table: the three own slots and the
# four options of an open offer (every option's trinket, not its price).
TRINKET_IDS = list(range(bgpy.SCALAR_TRINKET, bgpy.SCALAR_TRINKET + bgpy.TRINKET_SLOTS)) + [
    bgpy.SCALAR_TRINKET_OPTION + k * bgpy.TRINKET_OPTION_FEATURES for k in range(bgpy.TRINKET_CHOICES)
]
# Each opponent's hero (always visible), same hero embedding.
OPP_IDS = [bgpy.OPP_HERO]
# Their trinkets (visible), same trinket embedding.
OPP_TRINKET_IDS = list(range(bgpy.OPP_TRINKET, bgpy.OPP_TRINKET + bgpy.TRINKET_SLOTS))
# The minion's own id, the spell a Magicfin token was taught, and the ids of
# the effect-carrying cards Magnetized into it.
# ... and the Dark Gifts on it, which are effect card ids and ride in the
# same table.
SLOT_IDS = [bgpy.SLOT_CARD_ID, bgpy.SLOT_TAUGHT_SPELL] + list(
    range(bgpy.SLOT_ATTACHED, bgpy.SLOT_ATTACHED + bgpy.ATTACHED_SLOTS)) + list(
    range(bgpy.SLOT_GIFTS, bgpy.SLOT_GIFTS + bgpy.GIFT_SLOTS))
# Stats can run into the hundreds late (Holy Vanguard, Tyrael's 50/50, Blood
# Gems on a big Quilboar), so these are log-scaled after the division.
SLOT_LOG = [bgpy.SLOT_ATTACK, bgpy.SLOT_HEALTH, bgpy.SLOT_FIGHT_ATTACK, bgpy.SLOT_FIGHT_HEALTH,
            bgpy.SLOT_GEM_ATTACK, bgpy.SLOT_GEM_HEALTH]
# The same for scalars that grow without bound over a game.
SCALAR_LOG = [bgpy.SCALAR_DEATHRATTLES, bgpy.SCALAR_KNIGHT_DEATHS, bgpy.SCALAR_DISCARDED,
              bgpy.SCALAR_SPELLS_CAST, bgpy.SCALAR_UNDEAD_ATTACK, bgpy.SCALAR_UNDEAD_HEALTH,
              bgpy.SCALAR_DEITY_ATTACK, bgpy.SCALAR_DEITY_HEALTH, bgpy.SCALAR_LAST_DAMAGE]


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
    "GEM_ATTACK": 10, "GEM_HEALTH": 10, "BARRAGE_ATTACK": 10, "BARRAGE_HEALTH": 10,
    "REFRESH_ATTACK": 10, "REFRESH_HEALTH": 10, "BALLER": 10,
    "UNDEAD_ATTACK": 50, "UNDEAD_HEALTH": 50, "KNIGHT_DEATHS": 50,
    "ELEMENTALS_PLAYED": 20, "GOLDEN_PLAYED": 10, "GOLD_SPENT": 20,
    "DISCARDED": 20, "SPELLS_CAST": 20, "TRAILBLAZER_COVERS": 2,
    "SHARING_IS_CARING": 2, "LAST_DAMAGE": 15,
    "DEITY_ATTACK": 50, "DEITY_HEALTH": 50,
    "PENDING_SOURCE": 7, "PENDING_RUNS": 2, "PENDING_ATTACK": 20, "PENDING_HEALTH": 20,
    "SHOP_CONSUME_LEFT": 3, "DISCOVER_QUEUE": 3,
    "ARMOR": 25, "POWER_COST": 3, "POWER_USES_LEFT": 2, "POWER_USED_GAME": 5,
    "POWER2_COST": 3, "POWER2_USES_LEFT": 2, "POWER2_USED_GAME": 5,
    **{(bgpy.SCALAR_CHOOSE_ONE + j * bgpy.CHOOSE_FEATURES + 4, 2): 10 for j in range(2)},
})
OPP_SCALE = _scales(bgpy.OPP_SCALARS, "OPP_", {
    "HEALTH": 30, "TIER": 6, "TURNS_SINCE": 10, "PLACEMENT": 8, (bgpy.OPP_TRIBES, 10): 7,
    "DEITY_ATTACK": 50, "DEITY_HEALTH": 50, "GEM_ATTACK": 10, "GEM_HEALTH": 10,
    "SPELL_ATTACK": 10, "SPELL_HEALTH": 10, "ARMOR": 25,
})
OPP_LOG = [bgpy.OPP_DEITY_ATTACK, bgpy.OPP_DEITY_HEALTH]
SLOT_SCALE = _scales(SF, "SLOT_", {
    "ATTACK": 10, "HEALTH": 10, "FIGHT_ATTACK": 10, "FIGHT_HEALTH": 10, "TIER": 6,
    "MAGNETIZATIONS": 10, "VOLUMIZERS": 10, "IMPROVEMENTS": 10, "EVENTS_LEFT": 4,
    "WINNERS_BREAD": 2, "ACTIVATE_COST": 2, "EXTRA_MAGNETIZE": 2, "COST": 3,
    "GEM_ATTACK": 50, "GEM_HEALTH": 50, "LINK_GROUP": 64,
})

assert len(SCALAR_SCALE) == bgpy.SCALARS
assert len(OPP_SCALE) == bgpy.OPP_SCALARS
assert len(SLOT_SCALE) == SF


class Net(nn.Module):
    def __init__(self, n_cards=bgpy.N_CARDS, n_heroes=N_HERO_IDS,
                 n_trinkets=bgpy.N_TRINKETS, emb=32, slot=64, hidden=512):
        super().__init__()
        self.card = nn.Embedding(n_cards + 1, emb)  # 0 is an empty slot or no card
        self.hero = nn.Embedding(n_heroes + 1, emb)  # 0 is no hero
        self.trinket = nn.Embedding(n_trinkets + 1, emb)  # 0 is no trinket
        feats = [i for i in range(SF) if i not in SLOT_IDS]
        self.slot = nn.Sequential(nn.Linear(len(SLOT_IDS) * emb + len(feats), slot), nn.ReLU())
        n_slots = bgpy.TOTAL_SLOTS
        n_in = (n_slots * slot + bgpy.SCALARS
                + (len(SCALAR_IDS) + len(HERO_IDS) + len(TRINKET_IDS)) * emb
                + OPP * (bgpy.OPP_SCALARS + emb + len(OPP_TRINKET_IDS) * emb))
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
        keep[HERO_IDS] = 0.0
        keep[TRINKET_IDS] = 0.0
        self.register_buffer("s_scale", torch.tensor(SCALAR_SCALE, dtype=torch.float32))
        self.register_buffer("s_keep", keep)
        self.register_buffer("s_log", torch.tensor(SCALAR_LOG))
        self.register_buffer("o_scale", torch.tensor(OPP_SCALE, dtype=torch.float32))
        self.register_buffer("o_keep", torch.tensor(
            [0.0 if i in OPP_IDS or i in OPP_TRINKET_IDS else 1.0
             for i in range(bgpy.OPP_SCALARS)]))
        self.register_buffer("o_log", torch.tensor(OPP_LOG))
        self.register_buffer("m_scale", torch.tensor(SLOT_SCALE, dtype=torch.float32)[feats])
        self.register_buffer("feats", torch.tensor(feats, dtype=torch.long))
        self.register_buffer("log_cols", torch.tensor([feats.index(i) for i in SLOT_LOG]))

    def forward(self, obs):
        b = obs.shape[0]
        raw = obs[:, :bgpy.SCALARS]
        scalars = raw / self.s_scale * self.s_keep
        scalars[:, self.s_log] = torch.log1p(scalars[:, self.s_log].clamp(min=0))
        s_cards = self.card(raw[:, SCALAR_IDS].long()).reshape(b, -1)
        s_heroes = self.hero(raw[:, HERO_IDS].long()).reshape(b, -1)
        s_trinkets = self.trinket(raw[:, TRINKET_IDS].long()).reshape(b, -1)
        own = obs[:, bgpy.SCALARS:OWN_END].reshape(b, OWN_SLOTS, SF)
        opp = obs[:, OWN_END:].reshape(b, OPP, OPP_BLOCK)
        o = opp[:, :, :bgpy.OPP_SCALARS] / self.o_scale * self.o_keep
        o[:, :, self.o_log] = torch.log1p(o[:, :, self.o_log].clamp(min=0))
        o_heroes = self.hero(opp[:, :, OPP_IDS].long()).reshape(b, -1)
        o_trinkets = self.trinket(opp[:, :, OPP_TRINKET_IDS].long()).reshape(b, -1)
        opp_scalars = torch.cat([o.reshape(b, -1), o_heroes, o_trinkets], 1)
        opp_slots = opp[:, :, bgpy.OPP_SCALARS:].reshape(b, OPP * bgpy.BOARD, SF)
        slots = torch.cat([own, opp_slots], 1)
        n = slots.shape[1]
        cards = self.card(slots[:, :, SLOT_IDS].long()).reshape(b, n, -1)
        feats = slots[:, :, self.feats] / self.m_scale
        feats[:, :, self.log_cols] = torch.log1p(feats[:, :, self.log_cols].clamp(min=0))
        enc = self.slot(torch.cat([cards, feats], 2)).reshape(b, -1)
        h = self.trunk(torch.cat([enc, scalars, s_cards, s_heroes, s_trinkets, opp_scalars], 1))
        return self.policy(h), self.value(h).squeeze(1)


KIND_SIZES = [n for _, n in bgpy.ACTION_KINDS]
# The global actions after the trinket offer's options, which come from `s`
# alone: the Dark Discovery button (`ACTION_KINDS`'s last kind).
GLOBALS_AFTER = bgpy.ACTION_KINDS[-1][1]
assert sum(KIND_SIZES) == bgpy.N_ACTIONS


def kind_prior():
    """Starting logits that make every kind of action about equally likely,
    whatever its number of arguments. Uniform over all actions would almost
    never end a turn, since 42 of them are moves and 80 are plays."""
    return torch.cat([torch.full((n,), -float(torch.tensor(n).log())) for n in KIND_SIZES])


def masked(logits, mask):
    return logits.masked_fill(~mask, -1e9)


# The slot models (attn, pool) below.
#
# Token zones, in slot order: own Tavern minions, Tavern spell, hand, board,
# Discover options, then each opponent's last seen board (one zone each).
ZONES = [bgpy.SHOP, bgpy.SHOP_SPELL, bgpy.HAND, bgpy.BOARD, bgpy.DISCOVER] + [bgpy.BOARD] * OPP
ZONE_OF = [z for z, n in enumerate(ZONES) for _ in range(n)]
POS_OF = [i for n in ZONES for i in range(n)]
assert len(ZONE_OF) == bgpy.TOTAL_SLOTS
ZONE_START = [sum(ZONES[:z]) for z in range(len(ZONES))]
SHOP_T, SPELL_T, HAND_T, BOARD_T, DISC_T = ZONE_START[:5]
SHOP_R = slice(SHOP_T, SHOP_T + bgpy.SHOP)
HAND_R = slice(HAND_T, HAND_T + bgpy.HAND)
BOARD_R = slice(BOARD_T, BOARD_T + bgpy.BOARD)
DISC_R = slice(DISC_T, DISC_T + bgpy.DISCOVER)
# Move{from, to} with `to` skipping `from`: column of the 7x7 from/to grid.
MOVE_TO = [[t if t < f else t + 1 for t in range(bgpy.BOARD - 1)] for f in range(bgpy.BOARD)]


class SlotInputs(nn.Module):
    """Scales the observation like Net does and turns it into tokens of
    width d: one per card slot (card and taught-spell embeddings plus the
    slot's features through a small MLP, plus a learned embedding of its zone
    and of its position in the zone), one for my own scalars (with the last
    spell and pending card embedded) and one per opponent's scalars (plus
    that opponent's zone embedding, shared with its board)."""

    def __init__(self, n_cards, d, emb, n_heroes=N_HERO_IDS, n_trinkets=bgpy.N_TRINKETS):
        super().__init__()
        self.card = nn.Embedding(n_cards + 1, emb)
        self.hero = nn.Embedding(n_heroes + 1, emb)
        self.trinket = nn.Embedding(n_trinkets + 1, emb)  # 0 is no trinket
        feats = [i for i in range(SF) if i not in SLOT_IDS]
        self.enc = nn.Sequential(nn.Linear(len(SLOT_IDS) * emb + len(feats), d), nn.GELU(),
                                 nn.Linear(d, d))
        self.zone = nn.Embedding(len(ZONES), d)
        self.pos = nn.Embedding(max(ZONES), d)
        self.me = nn.Linear(bgpy.SCALARS + (len(SCALAR_IDS) + len(HERO_IDS) + len(TRINKET_IDS)) * emb, d)
        self.opp = nn.Linear(bgpy.OPP_SCALARS - len(OPP_IDS) - len(OPP_TRINKET_IDS)
                             + emb + len(OPP_TRINKET_IDS) * emb, d)
        keep = torch.ones(bgpy.SCALARS)
        keep[SCALAR_IDS] = 0.0
        keep[HERO_IDS] = 0.0
        keep[TRINKET_IDS] = 0.0
        self.register_buffer("s_scale", torch.tensor(SCALAR_SCALE, dtype=torch.float32))
        self.register_buffer("s_keep", keep)
        self.register_buffer("s_log", torch.tensor(SCALAR_LOG))
        self.register_buffer("o_scale", torch.tensor(OPP_SCALE, dtype=torch.float32))
        self.register_buffer("o_keep", torch.tensor(
            [0.0 if i in OPP_IDS or i in OPP_TRINKET_IDS else 1.0
             for i in range(bgpy.OPP_SCALARS)]))
        self.register_buffer("o_log", torch.tensor(OPP_LOG))
        self.register_buffer("o_cols", torch.tensor(
            [i for i in range(bgpy.OPP_SCALARS) if i not in OPP_IDS and i not in OPP_TRINKET_IDS]))
        self.register_buffer("m_scale", torch.tensor(SLOT_SCALE, dtype=torch.float32)[feats])
        self.register_buffer("feats", torch.tensor(feats, dtype=torch.long))
        self.register_buffer("log_cols", torch.tensor([feats.index(i) for i in SLOT_LOG]))
        self.register_buffer("zone_of", torch.tensor(ZONE_OF))
        self.register_buffer("pos_of", torch.tensor(POS_OF))
        self.register_buffer("opp_zone", torch.arange(5, 5 + OPP))

    def forward(self, obs):
        """(slots (B, 77, d), present (B, 77) bool, me (B, d), opp (B, OPP, d))"""
        b = obs.shape[0]
        raw = obs[:, :bgpy.SCALARS]
        scalars = raw / self.s_scale * self.s_keep
        scalars[:, self.s_log] = torch.log1p(scalars[:, self.s_log].clamp(min=0))
        s_cards = self.card(raw[:, SCALAR_IDS].long()).reshape(b, -1)
        s_heroes = self.hero(raw[:, HERO_IDS].long()).reshape(b, -1)
        s_trinkets = self.trinket(raw[:, TRINKET_IDS].long()).reshape(b, -1)
        own = obs[:, bgpy.SCALARS:OWN_END].reshape(b, OWN_SLOTS, SF)
        opp = obs[:, OWN_END:].reshape(b, OPP, OPP_BLOCK)
        o = opp[:, :, :bgpy.OPP_SCALARS] / self.o_scale * self.o_keep
        o[:, :, self.o_log] = torch.log1p(o[:, :, self.o_log].clamp(min=0))
        o_hero = self.hero(opp[:, :, OPP_IDS].long()).squeeze(2)
        o_trinkets = self.trinket(opp[:, :, OPP_TRINKET_IDS].long()).reshape(b, OPP, -1)
        opp_scalars = torch.cat([o[:, :, self.o_cols], o_hero, o_trinkets], 2)
        opp_slots = opp[:, :, bgpy.OPP_SCALARS:].reshape(b, OPP * bgpy.BOARD, SF)
        slots = torch.cat([own, opp_slots], 1)
        n = slots.shape[1]
        cards = self.card(slots[:, :, SLOT_IDS].long()).reshape(b, n, -1)
        feats = slots[:, :, self.feats] / self.m_scale
        feats[:, :, self.log_cols] = torch.log1p(feats[:, :, self.log_cols].clamp(min=0))
        tok = self.enc(torch.cat([cards, feats], 2)) + self.zone(self.zone_of) + self.pos(self.pos_of)
        present = slots[:, :, bgpy.SLOT_PRESENT] > 0
        me = self.me(torch.cat([scalars, s_cards, s_heroes, s_trinkets], 1))
        opp_tok = self.opp(opp_scalars) + self.zone(self.opp_zone)
        return tok, present, me, opp_tok


class ActionHead(nn.Module):
    """The N_ACTIONS logits, in the engine's order, from the own slot
    vectors and a summary vector s.

    Each own slot vector is mixed with s (z = GELU(W1 LN(t) + W2 s)), then:
    - buy i, target_shop i: from Tavern slot i's z
    - buy_spell: from the Tavern spell slot's z
    - sell j, activate j, target j: from board slot j's z
    - choose k: from Discover slot k's z, plus a term from s (Choose One
      options live in the scalars, not in a slot)
    - play hand k at position p: a term from hand slot k's z, plus a dot
      product of its query with the gap p (the board minions left and right
      of it, and a learned embedding of p)
    - discard k: from hand slot k's z
    - move from j to t: a term from board slot j's z, plus a dot product
      of j's query with t's key
    - roll, freeze, level, end turn, the two hero power buttons: from s
    - choose_trinket k: from s (the offer's options live in the scalars)
    Plus a per-action bias that starts at the kind prior, with the scoring
    weights small, so the starting policy is about Net's."""

    def __init__(self, d, hidden, qk=32):
        super().__init__()
        self.norm = nn.LayerNorm(d)
        self.tok = nn.Linear(d, hidden)
        self.ctx = nn.Linear(d, hidden)
        # columns: buy, buy_spell, sell, activate, target, target_shop,
        # choose, play, discard, move
        self.ptr = nn.Linear(hidden, 10)
        self.play_q = nn.Linear(hidden, qk)
        self.gap_l = nn.Linear(hidden, qk)
        self.gap_r = nn.Linear(hidden, qk)
        self.gap = nn.Parameter(torch.randn(bgpy.BOARD + 1, qk) * 0.1)
        self.move_q = nn.Linear(hidden, qk)
        self.move_k = nn.Linear(hidden, qk)
        # roll, freeze, level, end; the Choose One and power-choice columns
        # (CHOICES: a chooser offers up to four, a Discover three); the two
        # hero power buttons; the trinket offer's options; the Dark Discovery
        # button
        self.glob = nn.Linear(d, 6 + bgpy.CHOICES + bgpy.TRINKET_CHOICES + GLOBALS_AFTER)
        self.bias = nn.Parameter(kind_prior())
        for m in (self.ptr, self.play_q, self.move_q, self.glob):
            nn.init.orthogonal_(m.weight, 0.01)
            nn.init.zeros_(m.bias)
        self.scale = 1 / math.sqrt(qk)
        self.register_buffer("move_to", torch.tensor(MOVE_TO))

    def forward(self, slots, present, s):
        b = slots.shape[0]
        z = F.gelu(self.tok(self.norm(slots[:, :OWN_SLOTS])) + self.ctx(s)[:, None])
        p = self.ptr(z)
        g = self.glob(s)
        board = z[:, BOARD_R] * present[:, BOARD_R, None]
        hand = z[:, HAND_R]
        # gap p sits between board slots p-1 and p
        gaps = (F.pad(self.gap_l(board), (0, 0, 1, 0)) + F.pad(self.gap_r(board), (0, 0, 0, 1))
                + self.gap)
        play = p[:, HAND_R, 7, None] + torch.einsum(
            "bhq,bpq->bhp", self.play_q(hand), gaps) * self.scale
        move = p[:, BOARD_R, 9, None] + torch.einsum(
            "bfq,btq->bft", self.move_q(board), self.move_k(board)) * self.scale
        move = move.gather(2, self.move_to.expand(b, -1, -1))
        # A Discover's options sit in the Discover slots; a chooser's fourth
        # one (Bru'kan's Elements) has no slot, so it is the global column
        # after the three.
        choose = torch.cat([
            p[:, DISC_R, 6] + g[:, 4:4 + bgpy.DISCOVER],
            g[:, 4 + bgpy.DISCOVER:4 + bgpy.CHOICES],
        ], 1)
        logits = torch.cat([
            p[:, SHOP_R, 0],
            p[:, SPELL_T, 1:2],
            p[:, BOARD_R, 2],
            p[:, BOARD_R, 3],
            play.reshape(b, -1),
            p[:, BOARD_R, 4],
            p[:, SHOP_R, 5],
            choose,
            p[:, HAND_R, 8],
            move.reshape(b, -1),
            g[:, :4],
            g[:, 4 + bgpy.CHOICES:5 + bgpy.CHOICES],
            g[:, 5 + bgpy.CHOICES:6 + bgpy.CHOICES],
            g[:, 6 + bgpy.CHOICES:6 + bgpy.CHOICES + bgpy.TRINKET_CHOICES],
            # the Dark Discovery button, the last action of all
            g[:, 6 + bgpy.CHOICES + bgpy.TRINKET_CHOICES:],
        ], 1)
        return logits + self.bias


class Block(nn.Module):
    """Pre-norm transformer layer; `keep` (B, 1, 1, L) says which tokens can
    be attended to."""

    def __init__(self, d, heads, ff):
        super().__init__()
        self.heads = heads
        self.n1 = nn.LayerNorm(d)
        self.qkv = nn.Linear(d, 3 * d)
        self.out = nn.Linear(d, d)
        self.n2 = nn.LayerNorm(d)
        self.ff = nn.Sequential(nn.Linear(d, ff), nn.GELU(), nn.Linear(ff, d))

    def forward(self, x, keep):
        b, n, d = x.shape
        q, k, v = self.qkv(self.n1(x)).view(b, n, 3, self.heads, d // self.heads).permute(2, 0, 3, 1, 4)
        a = F.scaled_dot_product_attention(q, k, v, attn_mask=keep)
        x = x + self.out(a.transpose(1, 2).reshape(b, n, d))
        return x + self.ff(self.n2(x))


OPP_BOARDS = ("full", "pooled", "next")


def _pool(t, m):
    """Mean and max of tokens t (..., n, d) over the present ones m (..., n),
    0 when none is present: (..., 2d)."""
    m = m[..., None]
    cnt = m.sum(-2)
    mean = (t * m).sum(-2) / cnt.clamp(min=1)
    mx = t.masked_fill(~m, -1e9).max(-2).values * (cnt > 0)
    return torch.cat([mean, mx], -1)


class AttnNet(nn.Module):
    """Transformer over my scalars (the summary token), each opponent's
    scalars, and the card slots. Empty slots are masked as keys. Actions
    come from ActionHead on the final own slot tokens and the summary; the
    value from the summary.

    How opponent boards go in (`opp_boards`):
    - full: every opponent minion is a token (1 + 7 + 77 = 85 tokens).
    - pooled: each opponent's minions go through the same slot encoder, then
      mean and max over them, projected to d, are added to that opponent's
      token; only my 28 slots stay tokens (36 tokens).
    - next: pooled, plus the board of the opponent I face next as 7 full
      tokens with a learned "facing" embedding (43 tokens).

    A training minibatch of 8192 at 85 tokens is 700k tokens, and each layer
    keeps about 4 GB of activations for the backward pass in fp32, so the
    layers run in bf16 and, when gradients are on, are recomputed in the
    backward pass instead (about a third more compute, a fraction of the
    memory)."""

    def __init__(self, n_cards=bgpy.N_CARDS, d=128, layers=3, heads=4, ff=256, emb=32, bf16=True,
                 opp_boards="next"):
        super().__init__()
        assert opp_boards in OPP_BOARDS, opp_boards
        self.bf16 = bf16
        self.opp_boards = opp_boards
        self.inp = SlotInputs(n_cards, d, emb)
        self.blocks = nn.ModuleList(Block(d, heads, ff) for _ in range(layers))
        self.norm = nn.LayerNorm(d)
        self.head = ActionHead(d, d)
        self.value = nn.Sequential(nn.Linear(d, d), nn.GELU(), nn.Linear(d, 1))
        if opp_boards != "full":
            self.opp_pool = nn.Linear(2 * d, d)
        if opp_boards == "next":
            self.facing = nn.Parameter(torch.zeros(d))
            self.register_buffer("next_cols", OWN_END + torch.arange(OPP) * OPP_BLOCK + bgpy.OPP_NEXT)

    def compile_layers(self):
        """torch.compile each layer in place (parameter names unchanged)."""
        for blk in self.blocks:
            blk.compile(dynamic=True)

    def forward(self, obs):
        slots, present, me, opp = self.inp(obs)
        b = obs.shape[0]
        if self.opp_boards == "full":
            toks, keep = slots, present
        else:
            ob = slots[:, OWN_SLOTS:].reshape(b, OPP, bgpy.BOARD, -1)
            op = present[:, OWN_SLOTS:].reshape(b, OPP, bgpy.BOARD)
            opp = opp + self.opp_pool(_pool(ob, op))
            toks, keep = slots[:, :OWN_SLOTS], present[:, :OWN_SLOTS]
            if self.opp_boards == "next":
                nxt = obs[:, self.next_cols]
                i = nxt.argmax(1)
                ar = torch.arange(b, device=obs.device)
                toks = torch.cat([toks, ob[ar, i] + self.facing], 1)
                keep = torch.cat([keep, op[ar, i] & (nxt.sum(1, keepdim=True) > 0)], 1)
        x = torch.cat([me[:, None], opp, toks], 1)
        keep = torch.cat([keep.new_ones(b, 1 + OPP), keep], 1)[:, None, None]
        with torch.autocast("cuda", torch.bfloat16, enabled=self.bf16 and x.is_cuda):
            for blk in self.blocks:
                if torch.is_grad_enabled():
                    x = checkpoint(blk, x, keep, use_reentrant=False)
                else:
                    x = blk(x, keep)
        x = self.norm(x.float())
        s = x[:, 0]
        return self.head(x[:, 1 + OPP:], present, s), self.value(s).squeeze(1)


class PoolNet(nn.Module):
    """The slot tokens pooled per zone (mean and max over present slots, 0
    for an empty zone), concatenated with my scalar token and the opponent
    tokens into an MLP. Actions come from ActionHead on the slot tokens, with
    a summary projected from the MLP; the value from the MLP."""

    def __init__(self, n_cards=bgpy.N_CARDS, d=64, hidden=256, emb=32):
        super().__init__()
        self.inp = SlotInputs(n_cards, d, emb)
        n_in = len(ZONES) * 2 * d + d + OPP * d
        self.trunk = nn.Sequential(nn.Linear(n_in, hidden), nn.ReLU(),
                                   nn.Linear(hidden, hidden), nn.ReLU())
        self.summary = nn.Linear(hidden, d)
        self.head = ActionHead(d, hidden)
        self.value = nn.Linear(hidden, 1)

    def forward(self, obs):
        slots, present, me, opp = self.inp(obs)
        m = present[:, :, None]
        pools = []
        for start, n in zip(ZONE_START, ZONES):
            t, mz = slots[:, start:start + n], m[:, start:start + n]
            cnt = mz.sum(1)
            pools.append((t * mz).sum(1) / cnt.clamp(min=1))
            pools.append(t.masked_fill(~mz, -1e9).max(1).values * (cnt > 0))
        h = self.trunk(torch.cat([*pools, me, opp.flatten(1)], 1))
        return self.head(slots, present, self.summary(h)), self.value(h).squeeze(1)


MODELS = {"mlp": Net, "attn": AttnNet, "pool": PoolNet}
# Settings that change a model's shape, with their defaults; other models
# ignore them.
MODEL_OPTS = {"attn": {"opp_boards": "next"}}


def model_opts(kind, **given):
    """The shape settings `kind` uses: `given` where set, else the default."""
    return {k: given.get(k) or v for k, v in MODEL_OPTS.get(kind, {}).items()}


def build(kind, n_cards=bgpy.N_CARDS, compile=False, **opts):
    net = MODELS[kind](n_cards, **model_opts(kind, **opts))
    if compile and hasattr(net, "compile_layers"):
        net.compile_layers()
    return net


def ck_model(ck):
    """(kind, shape settings) of a ppo.py checkpoint or keep/ copy. Older
    ones have no model (mlp) or no opp_boards (attn was full then)."""
    cfg = ck.get("cfg", {})
    kind = ck.get("model", cfg.get("model", "mlp"))
    saved = ck.get("model_opts", {k: cfg[k] for k in MODEL_OPTS.get(kind, {}) if k in cfg})
    if kind == "attn":
        saved.setdefault("opp_boards", "full")
    return kind, model_opts(kind, **saved)


def load(path, dev="cpu", compile=False):
    """The network saved in a ppo.py checkpoint, built as the model it was
    trained with."""
    ck = torch.load(path, map_location=dev)
    kind, opts = ck_model(ck)
    net = build(kind, compile=compile, **opts).to(dev)
    net.load_state_dict(ck["net"])
    return net


def table_names(text):
    """Card names by id from a cards.toml text, in the engine's order:
    minions, then tokens, then spells (CardDb::parse)."""
    import tomllib
    raw = tomllib.loads(text)
    return [c["name"] for part in ("minion", "token", "spell") for c in raw.get(part, [])]


def _remap(src_w, own_w, old_names, new_names):
    """An embedding table for `new_names` with each name's row from `src_w`
    (laid out by `old_names`); row 0 (empty) is carried, new names keep
    `own_w`'s fresh rows. Returns (table, kept, new, dropped) counts."""
    w = own_w.clone()
    w[0] = src_w[0]
    old = {n: i for i, n in enumerate(old_names)}
    kept = 0
    for i, n in enumerate(new_names):
        j = old.get(n)
        if j is not None and j + 1 < src_w.shape[0]:
            w[i + 1] = src_w[j + 1]
            kept += 1
    dropped = len(set(old_names) - set(new_names))
    return w, kept, len(new_names) - kept, dropped


def warm_start(net, kind, path, cards=None, heroes=None, old_cards=None, old_heroes=None):
    """Load every entry of the checkpoint at `path` whose name and shape match
    `net` (a `kind` model). The card and hero embeddings are matched by name
    instead, row by row, so a card added, removed or moved in cards.toml
    keeps what the model knew about every other card: `cards` / `heroes` are
    this run's names by id, the checkpoint's own come from the checkpoint
    (`card_names`, `hero_names`) or `old_cards` / `old_heroes` for one saved
    before it stored them. Returns (loaded, fresh, skipped, src_opts,
    remapped) where remapped describes each table matched by name."""
    ck = torch.load(path, map_location="cpu")
    src_kind, src_opts = ck_model(ck)
    if src_kind != kind:
        raise SystemExit(f"--init-from {path} holds {src_kind}, not {kind}: "
                         "nothing to carry over")
    src = ck["net"]
    own = net.state_dict()
    take = {k: v for k, v in src.items() if k in own and own[k].shape == v.shape}
    remapped = []
    tables = (("card.weight", cards, ck.get("card_names") or old_cards),
              ("hero.weight", heroes, ck.get("hero_names") or old_heroes))
    for suffix, new_names, old_names in tables:
        if new_names is None or old_names is None:
            continue
        for k in own:
            if k.endswith(suffix) and k in src:
                w, kept, new, dropped = _remap(src[k], own[k], old_names, new_names)
                take[k] = w
                remapped.append(f"{k}: {kept} kept, {new} new, {dropped} dropped")
    net.load_state_dict(take, strict=False)
    params = [k for k, _ in net.named_parameters()]
    loaded = [k for k in params if k in take]
    fresh = [k for k in params if k not in take]
    skipped = [k for k in src if k not in take]
    if not loaded:
        raise SystemExit(f"--init-from {path}: no parameter matches by name and shape")
    return loaded, fresh, skipped, src_opts, remapped


SELL_A = slice(bgpy.SHOP + 1, bgpy.SHOP + 1 + bgpy.BOARD)


def _sample_obs(n=512, steps=300, seed=3):
    """Real observations from random legal play."""
    import numpy as np
    env = bgpy.VecEnv(n, seed, [4, 2, 1, 1, 0, 0])
    obs = np.zeros((n, bgpy.OBS_SIZE), np.float32)
    mask = np.zeros((n, bgpy.N_ACTIONS), bool)
    seat, kind = np.zeros(n, np.int64), np.zeros(n, np.int64)
    places = np.zeros((n, bgpy.PLAYERS), np.int8)
    tiers = np.zeros((n, bgpy.PLAYERS, bgpy.MAX_TIER), np.int8)
    tribes = np.zeros((n, bgpy.PLAYERS, bgpy.TRIBES), np.int8)
    reset = np.zeros(n, bool)
    rng = np.random.default_rng(seed)
    out = []
    for t in range(steps):
        env.observe(obs, mask, seat, kind)
        if t % 50 == 49:
            out.append(obs.copy())
        # random legal play, biased away from ending the turn so boards fill up
        w = mask * rng.random(mask.shape)
        w[:, bgpy.END_TURN] *= 0.05
        w[:, SELL_A] *= 0.05
        env.step(w.argmax(1), places, tiers, tribes, reset)
    return torch.from_numpy(np.concatenate(out))


def _swap(obs, zone_start, i, j):
    o = obs.clone()
    a = bgpy.SCALARS + (zone_start + i) * SF
    b = bgpy.SCALARS + (zone_start + j) * SF
    o[:, a:a + SF], o[:, b:b + SF] = obs[:, b:b + SF], obs[:, a:a + SF]
    return o


def check_pointers():
    """Per-slot logits come from the tokens of their slots: with the
    position embeddings zeroed (they are what tells slots apart otherwise),
    swapping two board minions swaps their sell, activate and target logits
    and permutes the move grid; swapping two Tavern minions swaps buy and
    target_shop; swapping two hand cards swaps their rows of play. Global
    logits and the value stay put."""
    torch.manual_seed(0)
    obs = _sample_obs()
    off = {}
    i = 0
    for name, n in bgpy.ACTION_KINDS:
        off[name] = slice(i, i + n)
        i += n
    present = obs[:, bgpy.SCALARS:OWN_END].reshape(len(obs), OWN_SLOTS, SF)[:, :, bgpy.SLOT_PRESENT] > 0
    cards = obs[:, bgpy.SCALARS:OWN_END].reshape(len(obs), OWN_SLOTS, SF)[:, :, bgpy.SLOT_CARD_ID]
    glob = list(range(off["roll"].start, bgpy.N_ACTIONS))
    for kind, opts in (("attn", {"opp_boards": "full"}), ("attn", {"opp_boards": "pooled"}),
                       ("attn", {"opp_boards": "next"}), ("pool", {})):
        net = build(kind, **opts).eval()
        with torch.no_grad():
            net.inp.pos.weight.zero_()
            for zs, kinds, label in ((BOARD_T, ("sell", "activate", "target"), "board"),
                                     (SHOP_T, ("buy", "target_shop"), "tavern"),
                                     (HAND_T, ("play", "discard"), "hand")):
                rows = present[:, zs] & present[:, zs + 1] & (cards[:, zs] != cards[:, zs + 1])
                x = obs[rows]
                assert len(x) >= 8, f"only {len(x)} samples with two different {label} cards"
                l0, v0 = net(x)
                l1, v1 = net(_swap(x, zs, 0, 1))
                for k in kinds:
                    a, b = l0[:, off[k]], l1[:, off[k]]
                    if k == "play":
                        a = a.reshape(-1, bgpy.HAND, bgpy.BOARD + 1)
                        b = b.reshape(-1, bgpy.HAND, bgpy.BOARD + 1)
                    perm = [1, 0] + list(range(2, a.shape[1]))
                    assert torch.allclose(a[:, perm], b, atol=1e-4), (kind, k)
                    assert (a[:, 0] - a[:, 1]).abs().max() > 1e-4, (kind, k, "vacuous")
                if label == "board":
                    grid0 = torch.full((len(x), bgpy.BOARD, bgpy.BOARD), float("nan"))
                    grid1 = grid0.clone()
                    to = torch.tensor(MOVE_TO)
                    for g, l in ((grid0, l0), (grid1, l1)):
                        g.scatter_(2, to.expand(len(x), -1, -1),
                                   l[:, off["move"]].reshape(-1, bgpy.BOARD, bgpy.BOARD - 1))
                    perm = [1, 0] + list(range(2, bgpy.BOARD))
                    assert torch.allclose(grid0[:, perm][:, :, perm], grid1, atol=1e-4, equal_nan=True), kind
                assert torch.allclose(l0[:, glob], l1[:, glob], atol=1e-4), (kind, label, "global")
                assert torch.allclose(v0, v1, atol=1e-4), (kind, label, "value")
        print(f"{kind} {opts}: pointer logits follow their slots")


if __name__ == "__main__":
    check_pointers()
