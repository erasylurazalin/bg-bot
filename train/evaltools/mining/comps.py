"""The comp table in docs/notes/mining/comps-overview.md.

Starts from the per-game labels in train/runs/comps-era-1106b/comps-era.toml
(the 77 games before 2026-11-07), which were rechecked against the final
boards here, then applies the corrections below and adds the six newer games.
"""
import os
import statistics
import sys
import tomllib

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import lib

PRIOR = os.path.join(lib.ROOT, 'train', 'runs', 'comps-era-1106b', 'comps-era.toml')

# Hands and engines we keep: the 3 to 5 cards that make the label recognisable.
DEFINE = {
    'Undead Reborn': ['Dead Bellringer', 'Drustfallen Butcher', 'Snazzy Phantom', 'Mummifier', 'Handless Forsaken'],
    'Aberration Deity spell-scaling': ['Nightmare Corroder', "Mysterious K'Thir", 'Faceless Converter', 'Dark Puppeteer', 'Gatekeeper Amalgam'],
    'Blood Gem spell-scaling': ['Hot-Air Surveyor', 'Gatekeeper Amalgam', 'Felboar', 'Balinda Stonehearth', 'Victorious Geomant'],
    'Demon Tavern-buff / consumption': ['Devilish Distractor', 'Twisted Wrathguard', 'Felboar', 'Balinda Stonehearth', 'Methodical Madness'],
    'Beast Beetles / Banana Slamma': ['Turquoise Skitterer', 'Ravaging Scorpid', 'Forest Rover', 'Buzzing Vermin', 'Banana Slamma'],
    'Mech Magnetize (Volumizers/Beatboxer/Spark Snapper)': ['Drone Duplicator', 'Spark Snapper', 'Auto Reveille', 'Polarizing Beatboxer', 'Green Volumizer'],
    'Glambot Blood Gem Mechs': ['Glambot', 'Utility Drone', 'Polarizing Beatboxer', 'Balinda Stonehearth', 'Gatekeeper Amalgam'],
    'Demon damage / Wrath Weaver': ['Wrath Weaver', 'Soul Rewinder', 'Tichondrius', 'Eredar Escapist', 'Ashen Corruptor'],
    'Beast Rally Deathrattle (Deathstrider/Titus)': ['Deathstrider', 'Titus Rivendare', 'Tasty Lobster', 'Headhunter Gryphon', 'Lurking Lionfish'],
    'Pirate Bounty': ['Sky Admiral Rogers', 'Clever Castaway', 'Proud Privateer', 'Bigwig Bandit', 'Hooktusk, Master Marauder'],
    'Beast Rally buffs (Lurking Lionfish)': ['Lurking Lionfish', 'Headhunter Gryphon', 'Flittering Bat', 'Tasty Lobster', 'Decoy Conjurer'],
    'Murloc Battlecry scaling': ['Brann Bronzebeard', 'Kelp Keeper', 'Magicfin Mycologist', 'Young Murk-Eye', 'Bubble Gunner'],
    'Elemental Tavern-buff': ['Living Azerite', 'Wildfire Elemental', 'Leyline Surfacer', 'Unbound Tempest', 'Balinda Stonehearth'],
    'Mech Magnetize into Felboar': ['Mechagnome Interpreter', 'Conveyor Construct', 'Trench Fighter', 'Felboar', 'Tyrael'],
    'Murloc hand-buff (Bream Counter)': ['Bream Counter', 'Diremuck Forager', 'Young Murk-Eye', 'Hackerfin', 'Drakkari Enchanter'],
    'Hooktusk Pirate Discover': ['Hooktusk, Master Marauder', 'Enterprising Escapee', 'Blade Collector', 'Clever Castaway', 'Treasure Parrot'],
    'Golden Pirate/Demon value': ['Maritime Extortionist', 'Aureate Laureate', 'Felfire Conjurer', 'Malchezaar, Prince of Dance', 'Soul Rewinder'],
    'Divine Shield Tavern-spell Attack scaling': ["Timecap'n Hooktail", 'Charging Czarina', 'Holy Vanguard', 'Relentless Deflector', 'Razorfen Flapper'],
    'Pirate economy into mixed Deathrattle (one-off)': ['Maritime Extortionist', 'Sky Admiral Rogers', 'Gearfin', 'Expert Aviator', 'Deathstrider'],
}

NEW = {
    'Hearthstone_2026_11_07_04_39_10-g1': 'Beast Rally Deathrattle (Deathstrider/Titus)',
    'Hearthstone_2026_11_07_04_39_10-g2': 'Undead Reborn',
    'Hearthstone_2026_11_08_03_26_06-g1': 'Pirate Bounty',
    'Hearthstone_2026_11_08_03_26_06-g2': 'Demon Tavern-buff / consumption',
    'Hearthstone_2026_11_08_03_26_06-g3': 'Aberration Deity spell-scaling',
    'Hearthstone_2026_11_08_03_26_06-g4': 'Beast Beetles / Banana Slamma',
}

SCAM = ['Hearthstone_2026_10_26_00_48_15-g2', 'Hearthstone_2026_10_26_00_48_15-g5',
        'Hearthstone_2026_11_05_15_51_51-g2']


def main():
    data = tomllib.load(open(PRIOR, 'rb'))
    rows = {}
    for c in data['comp']:
        if c['name'] == 'Murloc Venomous':
            continue
        ids = list(c['game_ids'])
        if c['name'] == 'Glambot Blood Gem Mechs':
            ids = [x for x in ids if x not in SCAM]
        if c['name'] == 'Aberration Deity spell-scaling':
            ids = [x for x in ids if x != 'Hearthstone_2026_10_28_02_28_32-g2']
        if c['name'] == 'Beast Beetles / Banana Slamma':
            ids = [x for x in ids if x != 'Hearthstone_2026_10_24_15_25_26-g5']
        if c['name'] == 'Beast Rally buffs (Lurking Lionfish)':
            ids.append('Hearthstone_2026_10_24_15_25_26-g5')
        rows[c['name']] = ids
    rows['Pirate economy into mixed Deathrattle (one-off)'] = ['Hearthstone_2026_10_28_02_28_32-g2']
    for gid, name in NEW.items():
        rows.setdefault(name, []).append(gid)
    games = {(g['source'], g['game']): g for g in lib.load_games()}
    print(f'{"comp":52s} {"games":>5s} {"avg":>4s} {"places":s}')
    for name, ids in sorted(rows.items(), key=lambda kv: (-len(kv[1]), kv[0])):
        places = [games[tuple(int(x) if x.isdigit() else x
                              for x in gid.rsplit('-g', 1))]['place'] for gid in ids]
        # the tuple above is awkward; rebuild from the id text
        print(f'{name:52s} {len(ids):5d} {statistics.mean(places):4.2f} {places}')
    print('\nassigned', sum(len(v) for v in rows.values()), 'scam', len(SCAM),
          'total', len(games))
    print('\nscam:', [(gid, games[(gid.rsplit("-g", 1)[0], int(gid.rsplit("-g", 1)[1]))]['place']) for gid in SCAM])
    print('\ndefining cards:')
    for name in sorted(rows):
        print(f'  {name}: {", ".join(DEFINE.get(name, []))}')


if __name__ == '__main__':
    main()
