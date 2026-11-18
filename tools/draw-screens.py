"""Editable SVG screens for FigCraft; all displayed data is synthetic."""
from pathlib import Path
from html import escape
import json

OUT = Path(__file__).resolve().parents[1] / "docs/design/list2cart"
OUT.mkdir(parents=True, exist_ok=True)
W, H = 1280, 900
INK, MUTED, BLUE, BG, LINE = "#142743", "#64748B", "#2168DD", "#F5F7FC", "#E1E7F0"
NAV = ["Pregled", "Košarica", "Planer i popisi", "Katalog artikala", "Trgovine i cijene", "Sinkronizacija"]

class Screen:
    def __init__(self, title, tab, subtitle):
        self.parts = []
        self.rect(0,0,W,H,BG,0)
        self.rect(0,0,224,H,"#EDF2FD",0)
        self.rect(24,28,42,42,BLUE,12)
        self.text(35,56,"L",25,"#FFFFFF",700)
        self.text(78,48,"List2Cart",22,INK,750)
        self.text(24,98,"Pametna kupnja.",13,MUTED)
        self.text(24,118,"Točan račun.",13,MUTED)
        for i, name in enumerate(NAV):
            y = 164+i*62
            if i == tab: self.rect(16,y,192,48,"#FFFFFF",12)
            self.circle(37,y+24,5,BLUE if i==tab else "#A8B6CF")
            self.text(54,y+29,name,14,BLUE if i==tab else MUTED,650 if i==tab else 500)
        self.card(16,740,192,132)
        self.circle(34,766,4,"#15976B")
        self.text(47,771,"Na ovom uređaju",12,INK,650)
        self.text(30,800,"Bez korisničkog računa.",11,MUTED)
        self.text(30,820,"Podaci ostaju lokalno.",11,MUTED)
        self.text(30,851,"List2Cart · Dizajn ekrana",10,MUTED)
        self.text(256,43,"OSOBNI PROSTOR / "+NAV[tab].upper(),11,MUTED,650)
        self.pill(1084,22,164,"Lokalni podaci","#E7F5EE","#11744E")
        self.text(256,99,title,29,INK,750)
        self.text(256,130,subtitle,14,MUTED)
        self.text(256,878,"PRIJEDLOG SUČELJA · IZMIŠLJENI PODACI · 17. 11. 2026.",10,MUTED)
    def rect(self,x,y,w,h,fill="#FFFFFF",r=12,stroke=None):
        self.parts.append(f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{r}" fill="{fill}"'+(f' stroke="{stroke}"' if stroke else '')+'/>')
    def card(self,x,y,w,h): self.rect(x,y,w,h,"#FFFFFF",18,LINE)
    def circle(self,x,y,r,c): self.parts.append(f'<circle cx="{x}" cy="{y}" r="{r}" fill="{c}"/>')
    def text(self,x,y,t,size=14,color=INK,weight=400):
        self.parts.append(f'<text x="{x}" y="{y}" font-family="Arial, sans-serif" font-size="{size}" font-weight="{weight}" fill="{color}">{escape(str(t))}</text>')
    def line(self,x,y,w): self.parts.append(f'<path d="M{x} {y}h{w}" stroke="{LINE}"/>')
    def button(self,x,y,w,label,primary=True):
        self.rect(x,y,w,42,BLUE if primary else "#FFFFFF",10,None if primary else LINE)
        self.text(x+16,y+27,label,13,"#FFFFFF" if primary else INK,650)
    def pill(self,x,y,w,label,bg="#EDF3FF",fg=BLUE):
        self.rect(x,y,w,28,bg,14); self.text(x+12,y+19,label,11,fg,650)
    def field(self,x,y,w,label,value):
        self.text(x,y,label,11,MUTED,650); self.rect(x,y+10,w,42,"#FFFFFF",9,LINE); self.text(x+12,y+36,value,13,INK)
    def row(self,y,name,sub,price,letter="",x=280,w=928):
        self.rect(x,y,44,44,"#EDF3FF",11); self.text(x+14,y+29,letter or name[0],21,BLUE,700)
        self.text(x+60,y+17,name,15,INK,650); self.text(x+60,y+39,sub,12,MUTED)
        self.text(x+w-100,y+28,price,17,INK,700); self.line(x,y+60,w)
    def body(self): return ''.join(self.parts)
    def svg(self): return f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="0 0 {W} {H}">{self.body()}</svg>'

screens=[]
def add(slug,s): screens.append((slug,s)); return s

s=add("01-pregled",Screen("Vaša kupnja, pod kontrolom.",0,"Košarica, planovi i spremljeni računi na jednom mjestu."))
s.card(256,168,992,274); s.pill(280,192,150,"AKTIVNA KOŠARICA")
s.text(280,259,"Super Konzum",20,INK,650); s.text(280,320,"18,42 €",52,INK,750)
s.text(282,352,"5 artikala · Spremno za provjeru na blagajni",14,MUTED)
s.button(280,379,236,"Otvori košaricu i skener →")
s.rect(931,207,260,168,"#EFF5FF",18); s.text(955,242,"Svaka stavka u cent.",19,BLUE,700)
for i,t in enumerate(["Skeniraj proizvod", "Provjeri količinu i cijenu", "Usporedi s računom"]): s.text(955,279+i*30,f"{i+1}. {t}",13,INK)
for x,k,v,sub in [(256,"Plan za subotu","5 artikala","Procjena: 19,10 €"),(594,"Katalog","128 artikala","Vaše zabilježene cijene"),(932,"Zadnji račun","18,42 €","Super Konzum · danas")]:
    s.card(x,466,316,176); s.text(x+24,503,k,14,MUTED,650); s.text(x+24,551,v,29,INK,750); s.text(x+24,586,sub,13,MUTED)
s.card(256,666,992,150); s.text(280,704,"Sljedeća kupnja",19,INK,700); s.text(280,739,"Subota · Lidl · Obiteljski popis",14,MUTED); s.button(996,722,224,"Otvori popis →",False)

s=add("02-kosarica",Screen("Provjerite svaki cent.",1,"Skenirajte artikle, upišite količine i usporedite iznos na blagajni."))
s.card(256,168,992,166); s.field(280,195,230,"TRGOVINA","Super Konzum"); s.button(930,194,290,"Skeniraj barkod kamerom")
for x,w,l,v in [(280,190,"BARKOD","3850100012345"),(486,282,"ARTIKL","Mlijeko 2,8 %"),(784,86,"KOLIČINA","2"),(886,92,"JEDINICA","kom"),(994,104,"CIJENA","1,19 €")]: s.field(x,265,w,l,v)
s.button(1114,275,106,"+ Dodaj")
s.card(256,358,992,282)
for y,n,sub,p in [(384,"Mlijeko 2,8 %","2 kom × 1,19 €","2,38 €"),(466,"Kava mljevena 250 g","1 kom × 4,49 €","4,49 €"),(548,"Jabuke Gala","1,250 kg × 1,89 €","2,36 €")]: s.row(y,n,sub,p)
s.card(256,664,992,162); s.text(280,701,"Ukupno u košarici",14,MUTED); s.text(280,751,"9,23 €",38,INK,750)
s.field(590,694,160,"IZNOS NA RAČUNU","9,23 €"); s.pill(778,708,220,"Iznosi se podudaraju","#E7F5EE","#11744E"); s.button(1006,748,214,"Spremi račun →")

s=add("03-planer",Screen("Planirajte sljedeću kupnju.",2,"Odvojeni popisi za trgovine i obitelj. Promjene stavki ostaju sačuvane."))
s.card(256,168,992,114); s.field(280,195,430,"NAZIV POPISA","Subotnja kupnja"); s.field(730,195,244,"TRGOVINA","Lidl"); s.button(994,205,226,"+ Kreiraj popis")
s.card(256,306,992,498); s.text(280,349,"Subotnja kupnja",24,INK,700); s.pill(1005,325,215,"Lidl · 2 od 5 kupljeno")
s.text(280,383,"Procijenjeni iznos",13,MUTED); s.text(280,424,"19,10 €",34,INK,750); s.line(280,448,940)
for i,(name,qty,price,done) in enumerate([("Mlijeko", "2 kom", "2,38 €",True),("Kruh", "1 kom","1,40 €",True),("Maslinovo ulje","1 L","7,99 €",False),("Jabuke","1,5 kg","2,84 €",False),("Kava","1 kom","4,49 €",False)]):
    y=477+i*56; s.rect(282,y-16,20,20,"#E7F5EE" if done else "#FFFFFF",5,LINE); s.text(286,y,"✓" if done else "",14,"#11744E",700); s.text(320,y,name,15,MUTED if done else INK,600); s.text(790,y,qty,13,MUTED); s.text(1100,y,price,15,INK,650)
s.button(980,748,240,"+ Dodaj stavku",False)

def catalog(detail=False):
    s=Screen("Vaš katalog artikala.",3,"Pretražite barkodove, usporedite vlastite cijene i pronađite proizvod.")
    s.card(256,168,992,112); s.field(280,195,688,"PRETRAGA","Pretraži naziv ili barkod…"); s.button(990,205,230,"+ Dodaj artikl")
    s.card(256,304,992,332)
    for y,n,sub,p in [(330,"Mlijeko 2,8 %","3850100012345 · Mliječni proizvodi","1,19 €"),(414,"Kava mljevena 250 g","3850100056789 · Napici","4,49 €"),(498,"Maslinovo ulje 1 L","3850100089012 · Ostalo","7,99 €")]: s.row(y,n,sub,p)
    if detail:
        s.rect(0,0,W,H,"#14274399",0)
        s.card(492,28,760,844); s.text(524,79,"Mlijeko 2,8 %",28,INK,750); s.text(524,109,"3850100012345 · Cijene koje ste zabilježili",13,MUTED)
        s.button(1096,52,124,"Zatvori",False); s.rect(524,141,116,116,"#EDF3FF",20); s.text(560,217,"M",48,BLUE,750)
        s.text(668,185,"Zadnja cijena",13,MUTED); s.text(668,229,"1,19 €",38,INK,750); s.text(524,308,"Usporedba po trgovinama",20,INK,700)
        for y,n,p,low in [(350,"Lidl","1,09 €",True),(438,"Super Konzum","1,19 €",False),(526,"Spar","1,29 €",False)]:
            s.text(524,y,n,16,INK,650); s.text(524,y+27,"Zabilježeno 17. 11. 2026.",12,MUTED); s.text(1100,y,p,20,INK,700); s.line(524,y+49,696)
            if low: s.pill(828,y-22,150,"Najniža cijena","#E7F5EE","#11744E")
        s.text(524,670,"Cijene su vaši lokalni zapisi, nisu online ponuda.",13,MUTED)
    return s
add("04-katalog",catalog()); add("07-detalj-artikla",catalog(True))

s=add("05-trgovine",Screen("Vaše trgovine.",4,"Organizirajte popise i zabilježene cijene po trgovinama."))
s.card(256,168,992,114); s.field(280,195,402,"NAZIV TRGOVINE","Nova trgovina…"); s.field(702,195,260,"LOKACIJA","Kvart ili grad…"); s.button(982,205,238,"+ Dodaj trgovinu")
for i,(n,loc,c,count) in enumerate([("Super Konzum","Zagreb · Radnička", "#D84E51","42 artikla"),("Lidl","Zagreb · Trešnjevka",BLUE,"36 artikala"),("Spar","Zagreb · Centar","#199D73","21 artikl"),("Kvartovska pekara","Zagreb · Trnje","#D49339","8 artikala")]):
    y=306+i*120; s.card(256,y,992,98); s.rect(280,y+24,48,48,c,13); s.text(296,y+57,n[0],24,"#FFFFFF",700); s.text(350,y+39,n,18,INK,700); s.text(350,y+66,loc+" · "+count,13,MUTED); s.button(1004,y+28,216,"Kreiraj popis →",False)

def sync(state="idle"):
    s=Screen("Razmjena između uređaja.",5,"Pratite stvarne korake prijenosa. Izvoz paketa nije potvrda primitka.")
    active=state in ("active","done","error")
    s.card(256,168,992,258 if active else 136)
    s.text(280,207,"Status razmjene" if not active else "Uvoz paketa",21,INK,700)
    if not active:
        s.pill(1070,185,150,"Nema prijenosa","#EEF2F7",MUTED); s.text(280,250,"Odaberite datoteku za uvoz ili izradite paket za drugi uređaj.",14,MUTED)
        y=328
    else:
        labels=["Čitanje datoteke","Provjera i primjena u bazu","Cjelovitost računa i osvježavanje prikaza"]
        for i,label in enumerate(labels):
            mark = "✓" if state=="done" or i==0 else ("!" if state=="error" and i==1 else "…" if i==1 else "○")
            s.circle(294,247+i*42,12,"#E7F5EE" if mark=="✓" else "#FFF1E8" if mark=="!" else "#EDF3FF")
            s.text(287,252+i*42,mark,14,"#11744E" if mark=="✓" else BLUE,700); s.text(320,252+i*42,label,14,INK if i<2 or state=="done" else MUTED,600)
        label={"active":"U tijeku","done":"Dovršeno","error":"Zaustavljeno"}[state]; s.pill(1068,185,156,label,"#E7F5EE" if state=="done" else "#FFF0EE" if state=="error" else "#EDF3FF","#11744E" if state=="done" else "#B64234" if state=="error" else BLUE)
        detail={"active":"Primjenjujemo podatke. Možete otvoriti drugi ekran.","done":"Uvezeno: 3 trgovine, 12 artikala i 2 popisa. Svi koraci su završeni.","error":"Paket nije podržan. Primjena nije završena; pogledajte izvor datoteke."}[state]
        s.text(280,389,detail,13,MUTED); y=450
    s.card(256,y,480,190); s.text(280,y+40,"Izvoz paketa",19,INK,700); s.text(280,y+72,"Izradite datoteku za ručni prijenos.",13,MUTED); s.button(280,y+110,204,"Izvezi osobni paket")
    s.card(760,y,488,190); s.text(784,y+40,"Uvoz paketa",19,INK,700); s.text(784,y+72,"Odaberite .list2cart.json datoteku.",13,MUTED); s.button(784,y+110,208,"Odaberi paket za uvoz",False)
    s.card(256,y+214,992,140); s.text(280,y+251,"Održavanje lokalne baze",18,INK,700); s.text(280,y+284,"Automatski, najviše jednom dnevno. Tombstone oznake ostaju sačuvane.",13,MUTED); s.button(1000,y+246,220,"Očisti stare verzije",False)
    return s
add("06-sinkronizacija",sync()); add("08-prijenos-u-tijeku",sync("active")); add("09-prijenos-dovrsen",sync("done")); add("10-prijenos-greska",sync("error"))

def duplicates(confirm=False):
    s=Screen("Pregled duplikata.",4,"Isti naziv ili barkod ne znači isti zapis. Izvorni identiteti ostaju sačuvani.")
    s.card(256,168,992,458 if confirm else 416); s.pill(280,191,176,"1 NERAZRIJEŠEN SUKOB","#FFF4DC","#916610")
    s.text(280,256,"Trgovine istog naziva: Lidl",23,INK,700)
    entries=[("Lidl · Trešnjevka","ebfabcb9-5f04-422a-b89c-bce3984bbb8b"),("Lidl · Zagreb","8ec8e7c6-d1c9-4357-a55b-3f1dc9faff8c")]
    if not confirm:
        for i,(n,id_) in enumerate(entries):
            y=294+i*110; s.rect(280,y,940,92,"#F8FAFD",12,LINE); s.text(300,y+31,n,18,INK,700); s.text(300,y+61,id_,12,MUTED); s.button(1020,y+24,180,"Zadrži ovaj zapis",False)
        s.text(280,552,"Prije spremanja prikazat ćemo što ostaje aktivno, a što se arhivira.",13,MUTED)
    else:
        s.text(280,303,"AKTIVAN OSTAJE",11,MUTED,700); s.text(280,337,entries[0][0],22,INK,700)
        s.text(280,389,"ARHIVIRAT ĆE SE",11,MUTED,700); s.text(280,421,entries[1][0],19,INK,650)
        s.text(280,469,"Računi, cijene i povijest ostaju vezani uz izvorne zapise.",14,MUTED)
        s.text(280,497,"Polja i povijest ne premještaju se na odabrani zapis.",14,MUTED)
        s.button(280,546,298,"Potvrdi i arhiviraj duplikat"); s.button(598,546,148,"Odustani",False)
    return s
add("11-duplikati",duplicates()); add("12-potvrda-duplikata",duplicates(True))

s=add("13-skener",Screen("Skenirajte barkod proizvoda.",1,"Kamera se koristi samo kada otvorite skener."))
s.rect(0,0,W,H,"#14274388",0); s.card(376,72,736,742); s.text(408,120,"Skeniraj proizvod",26,INK,750); s.button(958,94,122,"Zatvori",False)
s.rect(408,158,672,350,"#15243C",18); s.rect(546,242,396,164,"#203451",12,"#9ABAF4")
for i in range(45):
    x=575+i*7; s.rect(x,272,2 if i%3 else 4,102,"#FFFFFF",0)
s.rect(540,324,408,2,"#59A7FF",0); s.text(568,467,"Usmjerite kameru prema barkodu",14,"#C6D6EE")
s.button(408,534,318,"Uslikaj proizvod",False); s.button(746,534,334,"Odaberi fotografiju",False)
s.field(408,617,672,"ILI RUČNO UPIŠITE BARKOD","3850100012345"); s.button(826,708,254,"Primijeni barkod →")

s=add("14-racun-na-cekanju",Screen("Računi i dovršetak prijenosa.",0,"Nepotpuni računi ostaju sačuvani, a ne ulaze u dovršene rezultate."))
s.rect(256,168,992,106,"#FFF6E5",16,"#F0DCA9"); s.text(280,206,"1 račun čeka dovršetak prijenosa",20,"#886015",700)
s.text(280,240,"Prikazat će se kada stignu sve potvrđene stavke i zbrojevi budu usklađeni.",13,"#886015")
s.card(256,300,992,420); s.text(280,343,"Zadnji dovršeni račun",22,INK,700); s.pill(1038,319,182,"Sve stavke primljene","#E7F5EE","#11744E")
s.text(280,387,"Super Konzum · 17. 11. 2026.",14,MUTED)
for y,n,sub,p in [(424,"Mlijeko 2,8 %","2 kom × 1,19 €","2,38 €"),(506,"Kava mljevena 250 g","1 kom × 4,49 €","4,49 €")]: s.row(y,n,sub,p)
s.text(280,635,"Košarica: 6,87 €",21,INK,700); s.text(280,673,"Na računu: 6,87 € · Razlika: 0,00 €",14,"#11744E",650); s.button(1012,628,208,"Izvezi račun u Excel",False)

atlas=[]
cols=3; gap=80; margin=40; label=44
for i,(slug,screen) in enumerate(screens):
    (OUT/f"{slug}.svg").write_text(screen.svg())
    x=margin+(i%cols)*(W+gap); y=margin+(i//cols)*(H+gap+label)
    atlas.append(f'<text x="{x}" y="{y+25}" font-family="Arial" font-size="23" font-weight="700" fill="#142743">{escape(slug.replace("-"," "))}</text><g id="{slug}" transform="translate({x},{y+label})">{screen.body()}</g>')
aw=margin*2+cols*W+(cols-1)*gap; rows=(len(screens)+cols-1)//cols; ah=margin*2+rows*(H+label)+(rows-1)*gap
(OUT/"List2Cart-vsetky-ekrany.svg").write_text(f'<svg xmlns="http://www.w3.org/2000/svg" width="{aw}" height="{ah}" viewBox="0 0 {aw} {ah}"><rect width="100%" height="100%" fill="#DDE5F2"/>'+''.join(atlas)+'</svg>')
(OUT/"screens.json").write_text(json.dumps({"width":W,"height":H,"atlas_width":aw,"atlas_height":ah,"screens":[{"slug":slug,"x":margin+(i%cols)*(W+gap),"y":margin+(i//cols)*(H+gap+label)+label} for i,(slug,_) in enumerate(screens)]},indent=2))
print(f"Created {len(screens)} editable screens and {aw}×{ah} atlas in {OUT}")
