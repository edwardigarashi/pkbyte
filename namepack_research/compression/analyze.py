from pathlib import Path
from collections import Counter, defaultdict
import heapq, struct, json, math

ROOT=Path(__file__).resolve().parents[2]
OUTPUT=Path(__file__).resolve().parents[1]/'out/compression'
pack=(ROOT/'data/given_names_2024.npk').read_bytes()
names=(ROOT/'data/ssa_2024_given_names.txt').read_bytes().splitlines()
OUTPUT.mkdir(parents=True,exist_ok=True)
nsym=struct.unpack_from('<H',pack,36)[0]
ngroup,dlen=struct.unpack_from('<II',pack,40)
pos=64
dictionary=[]
for _ in range(nsym):
    size=pack[pos];pos+=1
    dictionary.append(pack[pos:pos+size]);pos+=size
assert pos==64+dlen
groups=[]
for _ in range(ngroup):
    codec,width,count=struct.unpack_from('<B3xIQ',pack,pos);pos+=16
    groups.append((codec,width,count))
rows=[]
for codec,width,count in groups:
    for _ in range(count):
        row=pack[pos:pos+width];pos+=width
        plain=row if codec==0 else b''.join(dictionary[c] for c in row)
        rows.append((codec,row,plain))
assert pos==len(pack)
assert set(plain for _,_,plain in rows)==set(names)

freq=Counter(c for codec,row,_ in rows if codec==1 for c in row)
heap=[(v,i,[i]) for i,v in freq.items()]
heapq.heapify(heap)
lengths={c:0 for c in freq}
serial=256
while len(heap)>1:
    a,_,ac=heapq.heappop(heap)
    b,_,bc=heapq.heappop(heap)
    for c in ac+bc:lengths[c]+=1
    heapq.heappush(heap,(a+b,serial,ac+bc));serial+=1
codes={}
code=0;prev=0
for c in sorted(lengths,key=lambda c:(lengths[c],c)):
    code <<= lengths[c]-prev
    codes[c]=(code,lengths[c]);code+=1;prev=lengths[c]

def serialize_huffman(items,tag):
    buckets=defaultdict(list)
    for codec,row,plain in items:
        if codec==0:
            width=8*len(row);value=int.from_bytes(row,'big')
        else:
            width=0;value=0
            for c in row:
                hc,hl=codes[c]
                value=(value<<hl)|hc;width+=hl
        buckets[(codec,width)].append((value,plain))
    payload=bytearray()
    decode={(v,l):dictionary[c] for c,(v,l) in codes.items()}
    minlen=min(lengths.values());maxlen=max(lengths.values())
    verified=[]
    for (codec,width),bucket in sorted(buckets.items()):
        bits=width*len(bucket)
        block=bytearray((bits+7)//8)
        # Deliberately simple bit writer and random-access decoder; not benchmarked.
        for rank,(value,plain) in enumerate(bucket):
            for k in range(width):
                if value>>(width-1-k)&1:
                    bit=rank*width+k;block[bit//8]|=1<<(7-bit%8)
        for rank,(_,plain) in enumerate(bucket):
            value=0
            for k in range(width):
                bit=rank*width+k
                value=(value<<1)|((block[bit//8]>>(7-bit%8))&1)
            if codec==0:
                result=value.to_bytes(width//8,'big')
            else:
                result=bytearray();remaining=width
                while remaining:
                    for hl in range(minlen,min(maxlen,remaining)+1):
                        prefix=value>>(remaining-hl)
                        if (prefix,hl) in decode:
                            result.extend(decode[(prefix,hl)])
                            remaining-=hl;value&=(1<<remaining)-1
                            break
                    else:raise AssertionError('Invalid code')
                result=bytes(result)
            assert result==plain
            verified.append(result)
        payload+=block
    assert set(verified)==set(names)
    # Explicit proposed accounting: header64, existing dictionary800,
    # one code-length byte per dictionary symbol, directory16 perbucket.
    total=64+dlen+nsym+16*len(buckets)+len(payload)
    result={'proposed_container_bytes':total,'actual_payload_bytes':len(payload),
            'status':'payload_built_and_exhaustively_verified; metadata_size_accounted_not_serialized',
            'header_bytes':64,'dictionary_bytes':dlen,'huffman_lengths_bytes':nsym,
            'directory_bytes':16*len(buckets),'groups':len(buckets),
            'verified_names':len(verified)}
    (OUTPUT/f'{tag}.payload').write_bytes(payload)
    return result

output={'current_bytes':len(pack),'names':len(names),'raw_name_bytes':sum(map(len,names)),
        'huffman_same_tokenization':serialize_huffman(rows,'huffman_fixed'),
        'huffman_code_lengths':dict(sorted(Counter(lengths.values()).items()))}

# Optimize each name under measured bit costs, using the same symbol dictionary
# and same Huffman model, not retraining or iterating its model.
starts=defaultdict(list)
for c,p in enumerate(dictionary):
    if c in lengths:starts[p[0]].append((p,c))
optimized=[]
for name in names:
    n=len(name);best=[math.inf]*(n+1);best[n]=0;choice=[None]*n
    for i in range(n-1,-1,-1):
        for p,c in starts[name[i]]:
            end=i+len(p)
            if name[i:end]==p and lengths[c]+best[end]<best[i]:
                best[i]=lengths[c]+best[end];choice[i]=(c,end)
    if best[0]>=n*8:
        optimized.append((0,name,name));continue
    codes_for_name=[];i=0
    while i<n:
        c,i=choice[i];codes_for_name.append(c)
    optimized.append((1,bytes(codes_for_name),name))
output['huffman_dp_reparse']=serialize_huffman(optimized,'huffman_dp')

# Prefix tries and minimal acyclic word graphs: exact state counts, not optimized
# serialization. A terminal marker is part of each state identity.
trie=[{}];terminal=set()
for name in names:
    q=0
    for b in name:
        if b not in trie[q]:trie[q][b]=len(trie);trie.append({})
        q=trie[q][b]
    terminal.add(q)
registry={};states=[];old_to_new={}
for q in reversed(range(len(trie))):
    arcs=tuple((b,old_to_new[dst]) for b,dst in sorted(trie[q].items()))
    key=(q in terminal,arcs)
    if key not in registry:registry[key]=len(states);states.append(key)
    old_to_new[q]=registry[key]
counts=[]
for term,arcs in states:
    counts.append(int(term)+sum(counts[dst] for b,dst in arcs))
assert counts[old_to_new[0]]==len(names)
def unrank(rank):
    q=old_to_new[0];out=bytearray()
    while True:
        term,arcs=states[q]
        if term:
            if rank==0:return bytes(out)
            rank-=1
        for b,dst in arcs:
            if rank<counts[dst]:out.append(b);q=dst;break
            rank-=counts[dst]
        else:raise AssertionError('Bad rank')
assert [unrank(i) for i in range(len(names))]==sorted(names)
output['dawg']={'trie_states':len(trie),'states':len(states),
                'arcs':sum(len(arcs) for _,arcs in states),
                'verified_names':len(names),'max_count':max(counts)}

# Elias-Fano sizing for monotone name-as-integer groups: exact logical bitvector
# lengths without select support. Map each position using an observed alphabet;
# mixed-radix value is invertible with those alphabet arrays.
efgroups=defaultdict(list)
for name in names:efgroups[len(name)].append(name)
ef=[]
for length,group in sorted(efgroups.items()):
    alphabets=[sorted(set(name[i] for name in group)) for i in range(length)]
    amap=[{c:i for i,c in enumerate(a)} for a in alphabets]
    values=[]
    for name in group:
        v=0
        for i,c in enumerate(name):v=v*len(alphabets[i])+amap[i][c]
        values.append(v)
    values.sort()
    # Translate by minimum; max+1 defines the smallest addressed universe.
    base=values[0];values=[v-base for v in values];universe=values[-1]+1
    low=max(0,(universe//len(values)).bit_length()-1)
    high_bits=(values[-1]>>low)+len(values)+1
    low_bits=len(values)*low
    ef.append({'length':length,'count':len(values),'universe':universe,
               'low_bits_each':low,'payload_bytes':(low_bits+7)//8+(high_bits+7)//8,
               'alphabet_bytes':sum(len(a)+1 for a in alphabets),
               'max_value_bits':universe.bit_length()})
output['ef_size_only']={'groups':ef,'payload_bytes':sum(g['payload_bytes'] for g in ef),
                        'alphabet_bytes':sum(g['alphabet_bytes'] for g in ef),
                        'select_index_not_included':True,'base_values_not_included':True}

path=OUTPUT/'analysis.json'
path.write_text(json.dumps(output,indent=2)+'\n')
print(json.dumps(output,indent=2))
