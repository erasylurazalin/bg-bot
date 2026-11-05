# Battlegrounds rules, as this engine plays them

This is Hearthstone Battlegrounds, not constructed Hearthstone. Ignore anything you know about constructed Hearthstone, and do not trust what you remember about Battlegrounds cards: use only the rules and card texts given here.

Nothing here is advice. It says what the rules are, not what to do.

## The lobby

- Eight players, each with one hero. A hero has 30 Health (a few have more) plus its own armor, and the armor differs by hero. Every point of damage to a hero takes armor first, then Health. Health and armor never come back on their own.
- Each lobby uses five of the ten minion types (Beast, Mech, Dragon, Murloc, Demon, Pirate, Elemental, Undead, Quilboar, Aberration) plus the neutral minions. A minion with two types is in the game if either type is in the lobby. You are told the lobby's five types.
- Everybody draws minions from one shared pool. Each minion has a fixed number of copies by tier: 16 for tier 1, then 15, 13, 11, 9 and 7 for tiers 2 to 6. Fewer copies left means worse odds of seeing it. A copy you buy leaves the pool while you own it; selling it puts it back.

## A turn

A turn has a shop phase, where every player buys and arranges their board at the same time, and then a fight phase, where each player fights one opponent. In this game only your decisions are yours: the other seven players are bots or real recorded boards, and you see their boards only when you fight them.

### Gold

- Turn 1 gives 3 Gold, every later turn gives 1 more, and the Gold you get per turn stops growing at 10.
- Unspent Gold is lost at the end of the turn.
- Buying a minion from the Tavern costs 3 Gold. Selling one gives 1 Gold back. Refreshing the Tavern (a roll) costs 1 Gold. Freezing is free.
- A Tavern spell costs the amount printed on it.
- Some cards, hero powers and trinkets change these prices or give extra Gold.

### Tavern tiers and upgrading

- Every player has a Tavern tier from 1 to 6. Tier 7 cards exist but you cannot level into tier 7.
- Upgrading one tier costs 5 Gold for tier 2, then 7, 8, 11 and 11 for tiers 3 to 6. The cost goes down by 1 at the start of every turn, down to 0, so waiting makes an upgrade cheaper.
- A higher tier shows more cards: 3 slots at tier 1, then 4, 4, 5, 5 and 6 slots at tiers 2 to 6. One more slot always holds a Tavern spell.
- The Tavern offers minions of your tier or lower only, drawn from the shared pool.

### Refresh and freeze

- The Tavern refreshes by itself at the start of each turn and at every roll.
- A frozen Tavern keeps its cards for the next turn; frozen slots that get emptied fill up again. Freezing costs nothing and can be undone.

### Hand and board

- Your hand holds 10 cards and your board 7 minions. A minion you buy goes to your hand.
- Playing a minion from hand puts it on the board at the position you choose and runs its Battlecry. Its position matters in the fight.
- A card you would get with a full hand is lost. A card that arrives at the start of your turn waits in a queue instead, and joins your hand when there is room.
- You may rearrange your board freely during your turn.
- A Tavern spell goes to your hand and is cast from there. A spell with no legal target cannot be cast.
- A turn is capped at 60 actions.

### Triples and golden minions

- Owning three copies of a minion (in hand or on board) merges them into one golden copy in your hand. Nothing triples during a fight: a third copy that appears there merges at the start of your next turn.
- A golden minion has double the printed stats plus the bonus stats of all three copies. It is a single card: it takes one board slot and one hand slot.
- A golden minion's own effect is doubled (twice the numbers, or the effect happens twice).
- Playing a golden minion from your hand gives you a Triple Reward: a 0 cost spell that Discovers a minion one tier above your Tavern tier at the moment you got it, capped at tier 6. Its tier is fixed when you get it.
- Every golden minion gives a Triple Reward when played from hand. The only exception is a golden whose own text says it gives none.

## Tavern spells

- One slot of every Tavern holds a spell. It refreshes and freezes with the minions and costs its printed cost.
- A spell does what its text says: it may need a target (your minions, the Tavern, or nothing), or it may trigger a Battlecry or Rally.

## Hero powers

- Every hero has one power. A passive power just works. A button power costs Gold and is pressed once per turn unless its text says otherwise ("Twice per turn", "Once per game"), and some are locked until a tier or a turn.
- Some powers work during the fight (at Start of Combat or when your minions attack).

## Trinkets

- Each player picks a Lesser trinket at the start of turn 6 and a Greater trinket at the start of turn 9, from an offer of 4. You pay the price shown.
- A trinket is a permanent passive effect for the rest of the game. Many work in fights.
- A trinket of a type or mechanic you already have on your board is more likely to be offered, and a tribe trinket you qualify for keeps its full price while one you fall short on costs less.

## Dark Gifts and the Dark Discovery button

- Every player has a Dark Discovery button from turn 3. Pressing it costs 3 Gold, works once per turn and 3 times per game.
- It opens an offer of three Tavern minions, each with a Dark Gift already on it; you take one minion and its gift. The gift stays on that minion for good, wherever the minion goes, and works the same on a golden minion.
- A gift is an extra permanent effect on one minion: extra stats, a keyword, a Deathrattle, a countdown, and so on.

## The season: trinkets, gifts and the Deity

- The Deity is a counter that awakens an Old God minion (C'Thun or Y'Shaarj) in the fight. It is shown as a number and a color.
- Quests, anomalies, buddies and duos are not in this game.

## The fight

Each player alive is paired with one other; with an odd number alive one player fights a ghost, which is a copy of a knocked out player's board.

Both boards are copied into the fight. Nothing in the fight changes your real board, except effects that say they are permanent.

### Order of events

1. **Start of Combat.** Illidan's Wingmen go first of all. Then hero powers, the Lesser trinket, the Greater trinket, and then minions from left to right. Between the two players the steps are interleaved: a coin flip picks who takes the first one, then they alternate.
2. **Attacks.** The side with more minions attacks first, and a coin flip decides it when both sides have the same number. The sides then alternate. A side attacks with its minions from left to right and wraps back to the left after its last one. A minion with 0 Attack never attacks. A minion with Windfury attacks twice in a row each time it is its turn to attack.
3. **Targets.** The defender is picked at random among the enemies. Taunt minions must be picked first while any are there, and Stealth minions cannot be picked. A Stealth minion loses Stealth when it attacks.
4. **Damage.** Both minions deal their Attack to each other at the same time. A Rally fires when its minion attacks, before the hit; if the Rally kills the target, the hit never lands and the attacker takes nothing back.
5. **Deaths.** After the attack, every minion at 0 Health or less dies. The dead are handled in board order, the attacking side first. Each dead minion runs its Deathrattle, then comes back for Reborn if it has it, and the whole step repeats until nothing more dies.
6. **The end.** The fight ends when one side has no minions. A side with minions left wins. If both sides are empty, or neither can attack, it is a tie.

A fight is capped at 500 attacks, after which it is a tie. Summons past the seventh minion are lost.

### Keywords

- **Taunt:** must be attacked first.
- **Divine Shield:** blocks the next damage of any size once, then is gone.
- **Windfury:** attacks twice in a row each time it is its turn to attack. Without Windfury a minion attacks once.
- **Venomous:** the next time it damages a minion, that minion dies. Venomous is then gone.
- **Poisonous:** like Venomous but permanent. Venomous and Poisonous are two different keywords and one minion can have both.
- **Reborn:** when it dies it comes back once with its printed Attack and 1 Health, and without Reborn.
- **Cleave:** its attack also hits the target's neighbors, which do not hit back.
- **Stealth:** cannot be attacked until it attacks.
- **Deathrattle:** an effect when the minion dies. Summons appear where the dead minion stood.
- **Avenge (N):** fires after N friendly minions die.
- **Rally:** fires when the minion attacks.
- **Magnetic:** a Mech (or the few cards whose text says otherwise) played directly to the left of a friendly Mech merges into it instead of taking a slot: the host gains its stats, keywords and effects. That counts as playing it.
- **Battlecry:** fires when the minion is played from hand. A golden Battlecry runs twice. A Battlecry does not fire when a minion is summoned or Discovered.
- **Spellcraft:** gives you a one turn spell while the minion is on your board.
- **Start of Combat / End of Turn:** when they fire, as their names say. Anything worded "at the end of your turn" is End of Turn; anything worded "next turn", "in 2 turns" or "at the start of" happens as a turn starts.
- **Bonus Keyword:** one of Taunt, Divine Shield, Windfury, Venomous, Reborn and Stealth. Magnetic and Poisonous are keywords but not Bonus Keywords, so "a random Bonus Keyword" never gives them.
- **"A minion of each type":** each minion gets at most one buff, and as many minions as possible get one.
- Golden minions' effects are doubled, not their keywords.
- Effects that count "cast a spell" count every spell (Tavern spells, Blood Gems, Triple Rewards). Effects that say "Tavern spell" count only Tavern spells.

## Damage to the hero, and the end of the game

- The loser of a fight takes damage equal to the winner's Tavern tier plus the Tavern tier of each minion the winner still has alive (tokens count, as their own tier).
- That damage is capped at 5 on turns 1 to 4, 10 on turns 5 to 7 and 15 from turn 8 on. Once 4 or fewer players are alive there is no cap. A tie deals no damage.
- A player at 0 Health or less is knocked out. Players knocked out in the same turn are ranked by the Health they had left, with ties broken at random.
- Placements are the order of the knockouts: the last player standing is 1st, the first knocked out is 8th.
- The game also ends at turn 40, and the players still alive are then ranked by Health.
- A knocked out player's minions go back to the pool.

## What you can see

- Your own Gold, tier, Health, armor, hand, board, Tavern, hero power, trinkets and Dark Gifts.
- For every opponent: their hero, Health, armor and Tavern tier, whether they are still alive, their placement once they are out, and the board they had the last time you fought them. You are also told which opponent you fight this turn.
- Nobody sees another player's hand, Gold or Tavern.

## The actions

One action at a time, from a list of the actions that are legal right now. Each action you are offered has a number and a description:

- buy a minion from the Tavern for its cost, or buy the Tavern spell;
- sell a minion from the board for 1 Gold;
- play a minion from hand to a board position, or cast a spell from hand;
- use a minion's Activate, if its text has one and you have not used it this turn;
- pick a target, when a spell, Battlecry, Activate or effect asks for one;
- pick an option, when a Discover, a Choose One, a hero offer, a trinket offer or a similar choice is open;
- move a minion to another position on your board;
- refresh the Tavern, freeze it or unfreeze it, upgrade your Tavern tier;
- press your hero power, or press the Dark Discovery button;
- end your turn, which starts your fight.
