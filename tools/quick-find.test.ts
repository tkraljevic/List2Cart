import {test,expect} from "bun:test";
import {CHAINS,archiveNote,branchName,branchesNear,chainForStoreName,chainLocative,chainName,chainSearchLabel,fold,matchLocalProducts,rankOffers,suggestPicks} from "../src/ui/commerce/quick-find";

test("chains from the cijene.dev archive get names, and a button label even without a known locative",()=>{
 expect(chainSearchLabel("tommy")).toBe("U Tommyju");
 expect(chainSearchLabel("metro")).toBe("Samo Metro");
 expect(chainName("trgovina-krk")).toBe("Trgovina Krk");
 expect(chainName("nova_trgovina")).toBe("nova trgovina");
 expect(chainForStoreName("Super Konzum")).toBe("konzum");
 expect(chainForStoreName("Tommy Split")).toBe("tommy");
 expect(chainForStoreName("Trgovina Krk Malinska")).toBe("trgovina-krk");
 expect(chainForStoreName("Pekara kod Ivana")).toBeNull();
 expect(chainForStoreName("Burek kod Admirala")).toBeNull();
 expect(chainForStoreName("dm drogerie")).toBe("dm");
});

test("only directly read chains are picked for the user; archive chains wait for their choice",()=>{
 const src=(chain:string,label:string,postal:string):PriceSource=>({chain,label,url:`https://x/${chain}/${label}`,price_date:"2026-11-19",branch_key:`${chain}:${label}`,postal_code:postal});
 const all:ChainSources[]=[{chain:"konzum",error:null,sources:[src("konzum","SUPER ZAGREB","10000")]},{chain:"tommy",error:null,sources:[src("tommy","supermarket Zagreb","10000")]}];
 expect(Object.keys(suggestPicks(all,"10000",[]))).toEqual(["konzum"]);
 expect(Object.keys(suggestPicks(all,"10000",[{chain:"tommy",branch_key:"tommy:supermarket Zagreb",label:""}])).sort()).toEqual(["konzum","tommy"]);
 const results=[{chain:"tommy",branch_label:"",error:null,results:{source:{chain:"tommy",label:"",url:"",price_date:"2026-11-19"},fetched_at:"",total_matches:0,products:[]}},{chain:"konzum",branch_label:"",error:null,results:{source:{chain:"konzum",label:"",url:"",price_date:"2026-11-19"},fetched_at:"",total_matches:0,products:[]}}];
 const note=archiveNote(results,v=>String(v));
 expect(note).toContain("Podaci za Tommy:");
 expect(note).toContain("cijene.dev");
 expect(note).toContain("19. 11. 2026.");
 expect(archiveNote(results.slice(1),v=>String(v))).toBe("");
});

test("every chain has a name to follow \"u\" on its search button",()=>{
 expect(CHAINS).toEqual(["konzum","lidl","kaufland","spar","plodine","eurospin","studenac"]);
 expect(CHAINS.map(chainLocative)).toEqual(["Konzumu","Lidlu","Kauflandu","Sparu","Plodinama","Eurospinu","Studencu"]);
});
import type {BranchSearch,ChainSources,PriceSource,RetailProduct} from "../src/api";
import type {Product} from "../src/types";

const product=(name:string,price:number,unit:number|null,basis:string|null,special:number|null=null):RetailProduct=>({name,barcode:null,brand:"",package:"",price_cents:price,special_price_cents:special,unit_price_cents:unit,unit_basis:basis});
const found=(chain:string,products:RetailProduct[]):BranchSearch=>({chain,branch_label:chain,error:null,results:{source:{chain,label:"",url:"",price_date:""},fetched_at:"",total_matches:products.length,products}});
const source=(chain:string,label:string,postal:string|null=null):PriceSource=>({chain,label,url:`https://${chain}/${label}`,price_date:"2026-11-19",branch_key:fold(label),postal_code:postal});

test("folding ignores case, diacritics and spaces, so lino finds LINO LADA",()=>{
 expect(fold("NAMAZ LINO LADA Čokoladni đ")).toBe("namazlinoladacokoladnid");
 const products=[{name:"LINO LADA DUO 400g"},{name:"Mlijeko"}] as Product[];
 expect(matchLocalProducts(products,"linolada").map(p=>p.name)).toEqual(["LINO LADA DUO 400g"]);
 expect(matchLocalProducts(products,"l")).toEqual([]);
});

test("cheapest per kilogram wins over a cheaper small package, and promotions count",()=>{
 const offers=rankOffers([
  found("konzum",[product("Lino 400 g",449,1123,"kg")]),
  found("lidl",[product("Lino 350 g",399,1140,"kg"),product("Lino 2,5 kg",1499,600,"kg",1299)]),
  found("spar",[product("Lino bez jedinice",199,null,null)]),
 ]);
 expect(offers.map(o=>o.product.name)).toEqual(["Lino 2,5 kg","Lino 400 g","Lino 350 g","Lino bez jedinice"]);
 expect(offers[0].priceCents).toBe(1299);
 expect(offers[0].unitPriceCents).toBe(520);
});

test("a failed chain does not hide the others",()=>{
 const offers=rankOffers([{chain:"konzum",branch_label:"",results:null,error:"Nema cjenika"},found("lidl",[product("Mlijeko",99,99,"L")])]);
 expect(offers.map(o=>o.chain)).toEqual(["lidl"]);
});

test("branches are found by town or by a nearby postal code",()=>{
 const all:ChainSources[]=[
  {chain:"konzum",error:null,sources:[source("konzum","SUPER ILICA 10000 ZAGREB 0101","10000"),source("konzum","SUPER SPLITSKA 21000 SPLIT","21000"),source("konzum","SUPER SESVETE 10360","10360")]},
  {chain:"spar",error:null,sources:[source("spar","supermarket zagreb jarun 0567")]},
 ];
 expect(branchesNear(all,"zagreb").get("konzum")!.map(s=>s.postal_code)).toEqual(["10000"]);
 expect(branchesNear(all,"Zagreb").get("spar")!.length).toBe(1);
 expect(branchesNear(all,"10000").get("konzum")!.map(s=>s.postal_code)).toEqual(["10000","10360"]);
 expect(branchesNear(all,"21000").get("konzum")!.map(s=>s.postal_code)).toEqual(["21000"]);
 const mixed:ChainSources[]=[
  {chain:"konzum",error:null,sources:[source("konzum","SUPER,ZAGREBAČKA AVENIJA 7 10000 ZAGREB,1000,108918,19.11.2026","10000")]},
  {chain:"lidl",error:null,sources:[source("lidl","Supermarket 104 Ulica 30 10410 Velika Gorica 1 19.11.2026 7.15h.csv","10410")]},
  {chain:"spar",error:null,sources:[source("spar","hipermarket zagreb avenija v. holjevca 62"),source("spar","supermarket split")]},
  {chain:"kaufland",error:null,sources:[source("kaufland","47432874968 Hipermarket Velika Gorica 5630")]},
 ];
 expect(branchesNear(mixed,"10000").get("spar")!.map(s=>s.label)).toEqual(["hipermarket zagreb avenija v. holjevca 62"]);
 expect(branchesNear(mixed,"10410").get("kaufland")!.length).toBe(1);
 const online:ChainSources[]=[{chain:"konzum",error:null,sources:[source("konzum","INTERNET ZAGREBAČKA AVENIJA 7 10000 ZAGREB","10000"),source("konzum","SUPER ZAGREB","10000")]}];
 expect(branchesNear(online,"zagreb").get("konzum")!.map(s=>s.label)).toEqual(["SUPER ZAGREB","INTERNET ZAGREBAČKA AVENIJA 7 10000 ZAGREB"]);
 const picks=suggestPicks(all,"zagreb",[]);
 expect(Object.keys(picks).sort()).toEqual(["konzum","spar"]);
});

test("a saved branch stays picked when the place is searched again",()=>{
 const all:ChainSources[]=[{chain:"konzum",error:null,sources:[source("konzum","A ZAGREB","10000"),source("konzum","B ZAGREB","10000")]}];
 expect(suggestPicks(all,"zagreb",[{chain:"konzum",branch_key:fold("B ZAGREB"),label:"B ZAGREB"}]).konzum).toBe("https://konzum/B ZAGREB");
});

test("branch names drop the price list date and time",()=>{
 expect(branchName("SUPER,ŽITNA 1A 10310 IVANIĆ GRAD,0204,108657,19.11.2026, 05-23.CSV")).toBe("SUPER ŽITNA 1A 10310 IVANIĆ GRAD 0204 108657");
 expect(branchName("Supermarket 104 Ulica Dr. Franje Tuđmana 30 10450 Jastrebarsko 1 19.11.2026 7.15h.csv")).toBe("Supermarket 104 Ulica Dr. Franje Tuđmana 30 10450 Jastrebarsko 1");
 expect(branchName("47432874968 Hipermarket Antuna Soljana 43 Zagreb 5630 cjenik robe 1 20261119 0730.csv")).toBe("47432874968 Hipermarket Antuna Soljana 43 Zagreb 5630 cjenik robe 1");
 expect(branchName("HIPERMARKET MIJE HALEUSA 1 10000 ZAGREB 055 553 18112026040401.csv")).toBe("HIPERMARKET MIJE HALEUSA 1 10000 ZAGREB 055 553");
 expect(branchName("Prodavaonica-Adamičeva ulica 19 ZAGREB-1113-554-2026-11-19-07-02-10-849227.xml")).toBe("Prodavaonica-Adamičeva ulica 19 ZAGREB-1113-554");
 expect(branchName("diskontna prodavaonica-310002-I Štefanovecki zavoj 12-Zagreb-10000-310002191126-19.11.2026-7.30.csv")).toBe("diskontna prodavaonica-310002-I Štefanovecki zavoj 12-Zagreb-10000-310002191126");
 expect(branchName("hipermarket zagreb avenija v. holjevca 62 8717 interspar 8717 zg buzin 0567 20261119 0330.csv")).toBe("hipermarket zagreb avenija v. holjevca 62 8717 interspar 8717 zg buzin 0567");
});
