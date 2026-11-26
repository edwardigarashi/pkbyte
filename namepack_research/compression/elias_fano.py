"""Research codec, not production format or hardened untrusted-input parser.

Mixed-radix name integers + Elias-Fano, grouped by original byte length.
64-byte header + 48-byte directory entries, per-position alphabets, packed lows,
unary highs, and 32-bit select positions every 64 names. All bytes accounted.
"""
from pathlib import Path
from collections import defaultdict
import struct, zlib, bisect, json

ROOT=Path(__file__).resolve().parents[2]
OUTPUT=Path(__file__).resolve().parents[1]/'out/compression'
DIR=struct.Struct('<HBBIQIIIIIIQ')
assert DIR.size==48

def wordbytes(value,bits):
    return value.to_bytes(((bits+63)//64)*8,'little')

def encode(names,quantum=64,power2=False):
    bylen=defaultdict(list)
    for name in names:bylen[len(name)].append(name)
    directory=bytearray();payload=bytearray();breakdown=defaultdict(int)
    assert quantum & (quantum-1)==0
    for length,group in sorted(bylen.items()):
        alphabets=[sorted(set(name[i] for name in group)) for i in range(length)]
        amaps=[{c:i for i,c in enumerate(a)} for a in alphabets]
        values=[]
        for name in group:
            value=0
            for i,c in enumerate(name):
                radix=1<<(len(alphabets[i])-1).bit_length() if power2 else len(alphabets[i])
                value=value*radix+amaps[i][c]
            assert value < 2**64, 'Need larger integer or fallback codec'
            values.append(value)
        values.sort()
        assert len(set(values))==len(values)
        base=values[0];values=[v-base for v in values]
        universe=values[-1]+1
        low=max(0,(universe//len(values)).bit_length()-1)
        highbits=(values[-1]>>low)+len(values)+1
        lopack=hipack=0;checkpoints=[]
        for rank,value in enumerate(values):
            lopack|=(value&((1<<low)-1))<<(rank*low)
            bit=(value>>low)+rank
            hipack|=1<<bit
            if rank%quantum==0:checkpoints.append(bit)
        alphadata=b''.join(struct.pack('<H',len(a))+bytes(a) for a in alphabets)
        lob=wordbytes(lopack,len(values)*low)
        hib=wordbytes(hipack,highbits)
        cpb=b''.join(struct.pack('<I',p) for p in checkpoints)
        directory+=DIR.pack(length,low,quantum.bit_length()-1,len(values),base,
                            len(alphadata),len(lob),len(hib),len(checkpoints),
                            highbits,int(power2),0)
        payload+=alphadata+lob+hib+cpb
        for k,v in [('alphabets',alphadata),('lows',lob),('highs',hib),('checkpoints',cpb)]:
            breakdown[k]+=len(v)
    header=bytearray(64)
    struct.pack_into('<8sIIQQQI',header,0,b'NAMESEF\0',1,len(bylen),len(names),
                     sum(map(len,names)),len(payload),0)
    data=header+directory+payload
    struct.pack_into('<I',data,40,zlib.crc32(data))
    breakdown['header']=64;breakdown['directory']=len(directory)
    assert sum(breakdown.values())==len(data)
    return bytes(data),dict(breakdown)

class Table:
    def __init__(self,data):
        magic,version,ngroups,count,totalraw,payload,crc=struct.unpack_from('<8sIIQQQI',data)
        assert magic==b'NAMESEF\0' and version==1
        check=bytearray(data);struct.pack_into('<I',check,40,0)
        assert zlib.crc32(check)==crc
        assert 64+48*ngroups+payload==len(data)
        pos=64+48*ngroups;self.groups=[];self.ends=[];self.count=count
        for g in range(ngroups):
            length,low,qlog,n,base,abytes,lbytes,hbytes,ncp,hbits,r1,r2=DIR.unpack_from(data,64+48*g)
            assert r1 in (0,1) and r2==0
            aend=pos+abytes;alphabets=[]
            for _ in range(length):
                k=struct.unpack_from('<H',data,pos)[0];pos+=2
                alphabets.append(data[pos:pos+k]);pos+=k
            assert pos==aend
            lows=data[pos:pos+lbytes];pos+=lbytes
            highs=data[pos:pos+hbytes];pos+=hbytes
            cp=struct.unpack_from('<'+'I'*ncp,data,pos);pos+=4*ncp
            self.groups.append((n,base,alphabets,low,lows,highs,cp,qlog,r1))
            self.ends.append(n+(self.ends[-1] if self.ends else 0))
        assert pos==len(data) and self.ends[-1]==count

    def get(self,rank):
        assert 0<=rank<self.count
        g=bisect.bisect_right(self.ends,rank)
        if g:rank-=self.ends[g-1]
        n,base,alphabets,low,lows,highs,cp,qlog,power2=self.groups[g]
        bit=cp[rank>>qlog];skip=rank&((1<<qlog)-1)
        word=bit//64
        chunk=int.from_bytes(highs[word*8:word*8+8],'little')&(-1<<(bit%64))
        scanned=1
        while chunk.bit_count()<=skip:
            skip-=chunk.bit_count();word+=1;scanned+=1
            chunk=int.from_bytes(highs[word*8:word*8+8],'little')
        for _ in range(skip):chunk&=chunk-1
        selected=word*64+(chunk&-chunk).bit_length()-1
        high=selected-rank
        p=rank*low
        lo=(int.from_bytes(lows[p//8:p//8+9],'little')>>(p%8))&((1<<low)-1)
        value=base+(high<<low)+lo
        result=bytearray(len(alphabets))
        for i in range(len(alphabets)-1,-1,-1):
            radix=1<<(len(alphabets[i])-1).bit_length() if power2 else len(alphabets[i])
            value,digit=divmod(value,radix)
            result[i]=alphabets[i][digit]
        assert value==0
        return bytes(result),scanned

if __name__=='__main__':
    names=(ROOT/'data/ssa_2024_given_names.txt').read_bytes().splitlines()
    OUTPUT.mkdir(parents=True,exist_ok=True)
    results=[]
    for quantum,power2 in [(q,p) for p in (False,True) for q in (16,32,64,128,256)]:
        data,breakdown=encode(names,quantum,power2)
        table=Table(data);decoded=[];scans=[]
        for rank in range(table.count):
            name,scanned=table.get(rank);decoded.append(name);scans.append(scanned)
        assert sorted(decoded)==sorted(names)
        assert len(decoded)==len(set(decoded))
        codec='efb' if power2 else 'ef'
        path=OUTPUT/f'ssa2024.{codec}{quantum}'
        path.write_bytes(data)
        results.append({'checkpoint_every':quantum,'power_of_two_radix':power2,'total_bytes':len(data),
                        'breakdown':breakdown,'verified_exact_names':len(decoded),
                        'select_word_scans_mean':sum(scans)/len(scans),
                        'select_word_scans_max':max(scans),
                        'select_word_scans_p99':sorted(scans)[int(len(scans)*.99)]})
    (OUTPUT/'ef_results.json').write_text(json.dumps(results,indent=2)+'\n')
    print(json.dumps(results,indent=2))
