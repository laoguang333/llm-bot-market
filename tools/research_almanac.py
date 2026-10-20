"""Market study behind Almanac: price response to news and drift by game phase.

Reads finished games (run/series/challenge output directories, one per seed) and prints
1. the average share of a news-implied value change already in the price, by ticks after
   first private disclosure (regression slope of the log price move on the value change);
2. the mean log return of an equal-weight buy-and-hold book per W-tick window, including
   dividends and, in the last window, the true terminal payout.
Uses hidden post-game data (true N, all news) for analysis only; the bot reads none of it.

    python tools/research_almanac.py runs/base/seed-*
"""
import sys
import numpy as np
import csv,json,collections
STOCKS=['T1','T2','T3','T4','C1','C2','C3','E1','E2','E3']
def load(run):
    d=json.load(open(run+'/summary.json'))
    T=d['ticks']; N=d['terminal_payments']
    P=np.full((T,10),np.nan); D=np.zeros((T,10)); V=np.zeros((T,10))
    idx={s:i for i,s in enumerate(STOCKS)}
    with open(run+'/bars.csv') as f:
        r=csv.reader(f); next(r)
        for row in r:
            t=int(row[0]); i=idx[row[1]]
            if row[5]: P[t,i]=int(row[5])
            D[t,i]=float(row[9]); V[t,i]=float(row[6])
    # forward fill price
    for i in range(10):
        last=np.nan
        for t in range(T):
            if np.isnan(P[t,i]): P[t,i]=last
            else: last=P[t,i]
    news=[]
    with open(run+'/news.csv') as f:
        for row in csv.DictReader(f):
            ch=[]
            for c in row['changes'].split(' '):
                if not c: continue
                s,v=c.split(':'); ch.append((idx[s],int(v)/50.0))
            news.append(dict(id=int(row['id']),layer=row['layer'],fr=int(row['first_reveal']),pub=int(row['public_tick']),eff=int(row['effective']),ch=ch))
    return dict(T=T,N=N,P=P,D=D,Vol=V,news=news,summary=d)


def curve(runs):    
    taus=list(range(0,61,10))+[61,62,63,65,70,80,90,100,110,120,130,150,180,220,300,500,800]
    X=[];Y=[];L=[]
    for run in runs:
        w=load(run);T=w['T'];P=w['P'];D=w['D']
        for n in w['news']:
            fr=n['fr']
            if fr<40 or fr+900>=T: continue
            for i,c in n['ch']:
                rem=(T-1-n['eff'])+5000
                base=max(D[fr,i],0.05)*((T-1-fr)+5000)
                x=c*rem/base
                if abs(x)<0.01: continue
                X.append(x); L.append(n['layer'])
                Y.append([np.log(P[fr+t,i]/P[fr-1,i]) for t in taus])
    X=np.array(X);Y=np.array(Y);L=np.array(L)
    for lay in ['all','Market','Sector','Stock']:
        m=np.ones(len(X),bool) if lay=='all' else (L==lay)
        bs=[np.polyfit(X[m],Y[m,j],1)[0] for j in range(len(taus))]
        print(lay,m.sum(),' '.join(f"{t}:{b:.2f}" for t,b in zip(taus,bs)))
    # nonlinearity: bins of x
    for lo,hi in [(0.01,0.03),(0.03,0.06),(0.06,0.12),(0.12,0.25),(0.25,9)]:
        m=(np.abs(X)>=lo)&(np.abs(X)<hi)
        j60=taus.index(60); j150=taus.index(150)
        r=np.sign(X[m])*(Y[m,j150]-Y[m,j60])
        print(f"|x| {lo}-{hi} n={m.sum()} mean|x|={np.abs(X[m]).mean():.3f} drift60-150 signed mean={r.mean():+.4f} ratio={r.mean()/np.abs(X[m]).mean():.2f} sd={r.std():.3f}")

def phase(runs, W=500):    
    M=[]
    for run in runs:
        w=load(run);T,N,P,D=w['T'],w['N'],w['P'],w['D']
        cumD=np.vstack([np.zeros(10),np.cumsum(D,axis=0)])
        out=[]
        for t in range(0,T-1,W):
            u=min(t+W,T-1)
            end = D[T-1]*50*N if u==T-1 else P[u]
            r=((cumD[u+1]-cumD[t+1])+end)/P[t]-1
            out.append(np.log(1+r).mean())
        M.append(out)
    M=np.array(M)
    for j in range(M.shape[1]):
        print(f"{j*W:5d} mean={M[:,j].mean():+.3f} t={M[:,j].mean()/M[:,j].std()*np.sqrt(len(M)):+.1f} pos={np.mean(M[:,j]>0):.2f}")

if __name__ == "__main__":
    runs = sys.argv[1:]
    curve(runs)
    phase(runs)
