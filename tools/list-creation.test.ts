import {test,expect} from "bun:test";
import {createListWithRefresh,listCreationError} from "../src/ui/commerce/list-creation";
import type {ShoppingList} from "../src/types";
const created:ShoppingList={id:42,sync_id:"new-list",name:"Nabavka",store_id:null,items:[],total_estimated_cents:0,updated_at:""};
test("creation surfaces denied private access without attempting a refresh",async()=>{
 let refreshed=false;
 await expect(createListWithRefresh("Nabavka",null,{create:async()=>{throw new Error("Privatni kanal dopusten je iskljucivo provjerenom vlastitom uredjaju istog principala");},refresh:async()=>{refreshed=true;return[];}})).rejects.toThrow("Privatni kanal");
 expect(refreshed).toBe(false);
 expect(listCreationError("Privatni kanal dopusten je iskljucivo")).toContain("Povežite ga ponovno");
 expect(listCreationError("Zajednicki popis moze stvoriti samo uredjaj koji upravlja mrezom")).toContain("uređaj koji upravlja mrežom");
 expect(listCreationError("Dijeliti popise moze samo uredjaj koji upravlja mrezom")).toBe("Dijeliti popise može samo uređaj koji upravlja mrežom.");
});
test("failed list refresh preserves confirmed create and never creates a duplicate",async()=>{
 let calls=0;
 const result=await createListWithRefresh(" Nabavka ",null,{create:async(name,store)=>{calls++;expect(name).toBe("Nabavka");expect(store).toBeNull();return created;},refresh:async()=>{throw new Error("refresh unavailable");}});
 expect(calls).toBe(1);expect(result.created).toEqual(created);expect(result.lists).toBeNull();expect(result.refreshError).toContain("refresh unavailable");
});
test("a shared list is asked for only when chosen",async()=>{
 const seen:boolean[]=[];
 const api={create:async(_n:string,_s:number|null,shared:boolean)=>{seen.push(shared);return created;},refresh:async()=>[created]};
 await createListWithRefresh("Vikend",null,api,true);
 await createListWithRefresh("Moj",null,api);
 expect(seen).toEqual([true,false]);
});
