import json,glob,collections,statistics as st,sys
tot=lambda b: sum(m['attack']+m['health'] for m in b)
TURNS=range(7,13)
per=collections.defaultdict(lambda: collections.Counter())   # turn -> counters
nturn=collections.Counter()
sell_origin=collections.Counter(); sell_names=collections.Counter(); sell_origin_names=collections.defaultdict(collections.Counter)
play_origin=collections.Counter(); gen_src=collections.Counter(); gen_kind=collections.Counter()
gold_gain_src=collections.Counter(); gold_spent=collections.Counter(); gold_gained=collections.Counter()
start_gold=[]; sold_strength=[]; held_age=[]
first_seen={}
ratio=[]
for f in sorted(glob.glob('boards/frozen/actions-2026-11-10/*.json')):
    d=json.load(open(f)); first_seen={}
    for t in d['turns']:
        T=t['turn']; s=t['start']
        for m in s['board']+s['hand']: first_seen.setdefault(m['id'],T-1)
        inr = T in TURNS
        if inr:
            nturn[T]+=1; start_gold.append(s['gold'])
        held={m['id'] for m in s['board']+s['hand']}
        bought=set(); gen={}
        prev=s; kinds={h['id']:h.get('kind','minion') for h in s['hand']}
        for a in t['actions']:
            step=a['step']; after=a.get('after',prev)
            for e in (a.get('entered') or []):
                src=(e.get('from') or {}).get('name','?')
                if src in ('Check Triples','3ofKindCheckPlayerEnchant'): 
                    gen[e['id']]='triple'; continue
                gen[e['id']]=src
                if inr and step!='buy': gen_src[src]+=1
            for h in after.get('hand',[]): kinds.setdefault(h['id'],h.get('kind','minion'))
            if step=='buy':
                bought.add(a['card']['id']); first_seen.setdefault(a['card']['id'],T)
            dg=after.get('gold',prev.get('gold',0))-prev.get('gold',0)
            if inr:
                key=step
                card=a.get('card') or {}
                if step=='play':
                    k=kinds.get(card.get('id'),'minion'); key='play_'+k
                    cid=card.get('id')
                    o='held' if cid in held else 'bought' if cid in bought else 'generated' if cid in gen else 'unknown'
                    play_origin[(k,o)]+=1
                if step=='buy' and a.get('spell'): key='buy_spell'
                per[T][key]+=1; per[T]['actions']+=1
                if dg<0: gold_spent[key]+= -dg
                if dg>0:
                    by=(a.get('gold_gain') or {}).get('by',{}).get('name') if a.get('gold_gain') else None
                    gold_gained[key]+=dg
                    gold_gain_src[(key, by or card.get('name') or '')]+=dg
                if step=='sell':
                    cid=card.get('id')
                    o='held' if cid in held else 'bought this turn' if cid in bought else ('made this turn: '+('triple' if gen.get(cid)=='triple' else 'generated')) if cid in gen else 'unknown'
                    sell_origin[o]+=1; sell_names[card.get('name')]+=1; sell_origin_names[o][card.get('name')]+=1
                    b=prev.get('board',[])
                    if b:
                        me=card.get('attack',0)+card.get('health',0); others=sorted(m['attack']+m['health'] for m in b)
                        sold_strength.append(sum(x<me for x in others)/max(1,len(others)-1) if len(others)>1 else 0)
                    if o=='held': held_age.append(T-first_seen.get(cid,T))
            prev=after
        if inr and s['board'] and t.get('end') and t['end'].get('board'):
            ratio.append(tot(t['end']['board'])/max(1,tot(s['board'])))
N=sum(nturn.values())
print('turns',dict(nturn),'total',N,'| start gold mean',round(st.mean(start_gold),2))
keys=sorted({k for T in per for k in per[T]})
print('per turn means (turns 7-12 pooled):')
pool=collections.Counter()
for T in per: pool.update(per[T])
for k in sorted(pool,key=lambda k:-pool[k]): print(f'   {k:14s} {pool[k]/N:6.2f}')
print('gold spent per turn by step:', {k:round(v/N,2) for k,v in gold_spent.most_common()}, 'total',round(sum(gold_spent.values())/N,2))
print('gold gained per turn by step:', {k:round(v/N,2) for k,v in gold_gained.most_common()}, 'total',round(sum(gold_gained.values())/N,2))
print('top gold gain sources (per turn):', [(k,round(v/N,2)) for k,v in gold_gain_src.most_common(25)])
print('sells per turn by origin:', {k:round(v/N,2) for k,v in sell_origin.most_common()})
for o,c in sell_origin_names.items(): print('   ',o,':',c.most_common(12))
print('sold card rank on board (0 weakest, 1 strongest): median',round(st.median(sold_strength),2),'mean',round(st.mean(sold_strength),2), 'share weakest third', round(sum(x<=0.34 for x in sold_strength)/len(sold_strength),2))
print('held-and-sold: turns held median',st.median(held_age),'share held 1 turn or less',round(sum(x<=1 for x in held_age)/len(held_age),2))
print('plays per turn by (kind, origin):', {str(k):round(v/N,2) for k,v in play_origin.most_common()})
print('generated cards per turn by source (top 30):', [(k,round(v/N,2)) for k,v in gen_src.most_common(30)], 'total',round(sum(gen_src.values())/N,2))
print('end/start board total ratio in-turn: median',round(st.median(ratio),2))
